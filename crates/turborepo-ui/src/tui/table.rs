use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Style, Stylize},
    text::Text,
    widgets::{Cell, Row, StatefulWidget, Table, TableState},
};
use tracing::debug;

use super::{
    event::TaskResult,
    spinner::SpinnerState,
    task::{Task, TasksByStatus},
    Error,
};

/// A widget that renders a table of their tasks and their current status
///
/// The table contains finished tasks, running tasks, and planned tasks rendered
/// in that order.
pub struct TaskTable {
    tasks_by_type: &TasksByStatus,
    pub scroll: TableState,
    spinner: SpinnerState,
    selected_index: &usize,
    user_has_interacted: &bool,
}

impl TaskTable {
    /// Construct a new table with all of the planned tasks
    pub fn new(
        tasks_by_type: &TasksByStatus,
        selected_index: &usize,
        user_has_interacted: &bool,
    ) -> Self {
        Self {
            selected_index,
            tasks_by_type,
            scroll: TableState::default(),
            spinner: SpinnerState::default(),
            user_has_interacted,
        }
    }

    // Provides a suggested width for the task table
    pub fn width_hint<'a>(tasks: impl Iterator<Item = &'a str>) -> u16 {
        let task_name_width = tasks
            .map(|task| task.len())
            .max()
            .unwrap_or_default()
            // Task column width should be large enough to fit "↑ ↓ to select task" instructions
            // and truncate tasks with more than 40 chars.
            .clamp(13, 40) as u16;
        // Add space for column divider and status emoji
        task_name_width + 1
    }

    /// Update the current time of the table
    pub fn tick(&mut self) {
        self.spinner.update();
    }

    pub fn tasks_started(&self) -> Vec<&str> {
        let (errors, success): (Vec<_>, Vec<_>) = self
            .tasks_by_type
            .finished
            .iter()
            .partition(|task| matches!(task.result(), TaskResult::Failure));

        // We return errors last as they most likely have information users want to see
        success
            .into_iter()
            .map(|task| task.name())
            .chain(self.tasks_by_type.running.iter().map(|task| task.name()))
            .chain(errors.into_iter().map(|task| task.name()))
            .collect()
    }

    fn finished_rows(&self) -> impl Iterator<Item = Row> + '_ {
        self.tasks_by_type.finished.iter().map(move |task| {
            Row::new(vec![
                Cell::new(task.name()),
                Cell::new(match task.result() {
                    TaskResult::Success => Text::raw("✔").style(Style::default().light_green()),
                    TaskResult::Failure => Text::raw("✘").style(Style::default().red()),
                }),
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

    /// Convenience method which renders and updates scroll state
    pub fn stateful_render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let mut scroll = self.scroll.clone();
        self.spinner.update();
        frame.render_stateful_widget(&*self, area, &mut scroll);
        self.scroll = scroll;
    }
}

impl<'a> StatefulWidget for &'a TaskTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer, state: &mut Self::State) {
        let width = area.width;
        let active_index = self.selected_index;
        let user_has_interacted = self.user_has_interacted;
        let bar = "─".repeat(usize::from(width));
        let table = Table::new(
            self.running_rows()
                .chain(self.planned_rows())
                .chain(self.finished_rows()),
            [
                Constraint::Min(14),
                // Status takes one cell to render
                Constraint::Length(1),
            ],
        )
        .highlight_style(Style::default().fg(Color::Yellow))
        .column_spacing(0)
        .header(
            vec![
                format!("Tasks {active_index} {user_has_interacted}\n{bar}"),
                " \n─".to_owned(),
            ]
            .into_iter()
            .map(Cell::from)
            .collect::<Row>()
            .height(2),
        )
        .footer(
            vec![format!("{bar}\n↑ ↓ to navigate"), "─\n ".to_owned()]
                .into_iter()
                .map(Cell::from)
                .collect::<Row>()
                .height(2),
        );
        StatefulWidget::render(table, area, buf, state);
    }
}
//
// #[cfg(test)]
// mod test {
//     use super::*;
//
//     #[test]
//     fn test_scroll() {
//         let mut table = TaskTable::new(vec![
//             "foo".to_string(),
//             "bar".to_string(),
//             "baz".to_string(),
//         ]);
//         assert_eq!(table.scroll.selected(), None, "starts with no
// selection");         table.next();
//         assert_eq!(table.scroll.selected(), Some(0), "scroll starts from 0");
//         table.previous();
//         assert_eq!(table.scroll.selected(), Some(0), "scroll stays in
// bounds");         table.next();
//         table.next();
//         assert_eq!(table.scroll.selected(), Some(2), "scroll moves
// forwards");         table.next();
//         assert_eq!(table.scroll.selected(), Some(2), "scroll stays in
// bounds");     }
//
//     #[test]
//     fn test_selection_follows() {
//         let mut table = TaskTable::new(vec!["a".to_string(), "b".to_string(),
// "c".to_string()]);         table.next();
//         table.next();
//         assert_eq!(table.scroll.selected(), Some(1), "selected b");
//         assert_eq!(table.selected(), Some("b"), "selected b");
//         table.start_task("b").unwrap();
//         assert_eq!(table.scroll.selected(), Some(0), "b stays selected");
//         assert_eq!(table.selected(), Some("b"), "selected b");
//         table.start_task("a").unwrap();
//         assert_eq!(table.scroll.selected(), Some(0), "b stays selected");
//         assert_eq!(table.selected(), Some("b"), "selected b");
//         table.finish_task("a", TaskResult::Success).unwrap();
//         assert_eq!(table.scroll.selected(), Some(1), "b stays selected");
//         assert_eq!(table.selected(), Some("b"), "selected b");
//     }
//
//     #[test]
//     fn test_restart_task() {
//         let mut table = TaskTable::new(vec!["a".to_string(), "b".to_string(),
// "c".to_string()]);         table.next();
//         table.next();
//         // Start all tasks
//         table.start_task("b").unwrap();
//         table.start_task("a").unwrap();
//         table.start_task("c").unwrap();
//         assert_eq!(table.get(0), Some("b"), "b is on top (running)");
//         table.finish_task("a", TaskResult::Success).unwrap();
//         assert_eq!(
//             (table.get(0), table.get(1)),
//             (Some("a"), Some("b")),
//             "a is on top (done), b is second (running)"
//         );
//
//         table.finish_task("b", TaskResult::Success).unwrap();
//         assert_eq!(
//             (table.get(0), table.get(1)),
//             (Some("a"), Some("b")),
//             "a is on top (done), b is second (done)"
//         );
//
//         // Restart b
//         table.start_task("b").unwrap();
//         assert_eq!(
//             (table.get(1), table.get(2)),
//             (Some("c"), Some("b")),
//             "b is third (running)"
//         );
//
//         // Restart a
//         table.start_task("a").unwrap();
//         assert_eq!(
//             (table.get(0), table.get(1), table.get(2)),
//             (Some("c"), Some("b"), Some("a")),
//             "c is on top (running), b is second (running), a is third
// (running)"         );
//     }
//
//     #[test]
//     fn test_selection_stable() {
//         let mut table = TaskTable::new(vec!["a".to_string(), "b".to_string(),
// "c".to_string()]);         table.next();
//         table.next();
//         assert_eq!(table.scroll.selected(), Some(1), "selected b");
//         assert_eq!(table.selected(), Some("b"), "selected b");
//         // start c which moves it to "running" which is before "planned"
//         table.start_task("c").unwrap();
//         assert_eq!(table.scroll.selected(), Some(2), "selection stays on b");
//         assert_eq!(table.selected(), Some("b"), "selected b");
//         table.start_task("a").unwrap();
//         assert_eq!(table.scroll.selected(), Some(2), "selection stays on b");
//         assert_eq!(table.selected(), Some("b"), "selected b");
//         // c
//         // a
//         // b <-
//         table.previous();
//         table.previous();
//         assert_eq!(table.scroll.selected(), Some(0), "selected c");
//         assert_eq!(table.selected(), Some("c"), "selected c");
//         table.finish_task("a", TaskResult::Success).unwrap();
//         assert_eq!(table.scroll.selected(), Some(1), "c stays selected");
//         assert_eq!(table.selected(), Some("c"), "selected c");
//         table.previous();
//         table.finish_task("c", TaskResult::Success).unwrap();
//         assert_eq!(table.scroll.selected(), Some(0), "a stays selected");
//         assert_eq!(table.selected(), Some("a"), "selected a");
//     }
// }
