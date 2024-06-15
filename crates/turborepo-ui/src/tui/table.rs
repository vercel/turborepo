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
    task::{Finished, Planned, Running, Task},
    Error,
};

/// A widget that renders a table of their tasks and their current status
///
/// The table contains finished tasks, running tasks, and planned tasks rendered
/// in that order.
pub struct TaskTable {
    // Tasks to be displayed
    // Ordered by when they finished
    finished: Vec<Task<Finished>>,
    // Ordered by when they started
    running: Vec<Task<Running>>,
    // Ordered by task name
    planned: Vec<Task<Planned>>,
    // State used for showing things
    scroll: TableState,
    spinner: SpinnerState,
}

impl TaskTable {
    /// Construct a new table with all of the planned tasks
    pub fn new(tasks: impl IntoIterator<Item = String>) -> Self {
        let mut planned = tasks.into_iter().map(Task::new).collect::<Vec<_>>();
        planned.sort_unstable();
        planned.dedup();
        Self {
            planned,
            running: Vec::new(),
            finished: Vec::new(),
            scroll: TableState::default(),
            spinner: SpinnerState::default(),
        }
    }

    // Provides a suggested width for the task table
    pub fn width_hint<'a>(tasks: impl Iterator<Item = &'a str>) -> u16 {
        let task_name_width = tasks
            .map(|task| task.len())
            .max()
            .unwrap_or_default()
            // Task column width should be large enough to fit "Task" title
            // and truncate tasks with more than 40 chars.
            .clamp(4, 40) as u16;
        // Add space for column divider and status emoji
        task_name_width + 1
    }

    /// Number of rows in the table
    pub fn len(&self) -> usize {
        self.finished.len() + self.running.len() + self.planned.len()
    }

    /// If there are no tasks in the table
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Mark the given task as started.
    /// If planned, pulls it from planned tasks and starts it.
    /// If finished, removes from finished and starts again as new task.
    pub fn start_task(&mut self, task: &str) -> Result<(), Error> {
        if let Ok(planned_idx) = self
            .planned
            .binary_search_by(|planned_task| planned_task.name().cmp(task))
        {
            let planned = self.planned.remove(planned_idx);
            let old_row_idx = self.finished.len() + self.running.len() + planned_idx;
            let new_row_idx = self.finished.len() + self.running.len();
            let running = planned.start();
            self.running.push(running);

            if let Some(selected_idx) = self.scroll.selected() {
                // If task that was just started is selected, then update selection to follow
                // task
                if selected_idx == old_row_idx {
                    self.scroll.select(Some(new_row_idx));
                } else if new_row_idx <= selected_idx && selected_idx < old_row_idx {
                    // If the selected task is between the old and new row positions
                    // then increment the selection index to keep selection the same.
                    self.scroll.select(Some(selected_idx + 1));
                }
            }
        } else if let Some(finished_idx) = self
            .finished
            .iter()
            .position(|finished_task| finished_task.name() == task)
        {
            let finished = self.finished.remove(finished_idx);
            let old_row_idx = finished_idx;
            let new_row_idx = self.finished.len() + self.running.len();
            let running = Task::new(finished.name().to_string()).start();
            self.running.push(running);

            if let Some(selected_idx) = self.scroll.selected() {
                // If task that was just started is selected, then update selection to follow
                // task
                if selected_idx == old_row_idx {
                    self.scroll.select(Some(new_row_idx));
                } else if new_row_idx <= selected_idx && selected_idx < old_row_idx {
                    // If the selected task is between the old and new row positions
                    // then increment the selection index to keep selection the same.
                    self.scroll.select(Some(selected_idx + 1));
                }
            }
        } else {
            debug!("could not find '{task}' to start");
            return Err(Error::TaskNotFound { name: task.into() });
        }

        self.tick();
        Ok(())
    }

    /// Mark the given running task as finished
    /// Errors if given task wasn't a running task
    pub fn finish_task(&mut self, task: &str, result: TaskResult) -> Result<(), Error> {
        let running_idx = self
            .running
            .iter()
            .position(|running| running.name() == task)
            .ok_or_else(|| {
                debug!("could not find '{task}' to finish");
                Error::TaskNotFound { name: task.into() }
            })?;
        let old_row_idx = self.finished.len() + running_idx;
        let new_row_idx = self.finished.len();
        let running = self.running.remove(running_idx);
        self.finished.push(running.finish(result));

        if let Some(selected_row) = self.scroll.selected() {
            // If task that was just started is selected, then update selection to follow
            // task
            if selected_row == old_row_idx {
                self.scroll.select(Some(new_row_idx));
            } else if new_row_idx <= selected_row && selected_row < old_row_idx {
                // If the selected task is between the old and new row positions then increment
                // the selection index to keep selection the same.
                self.scroll.select(Some(selected_row + 1));
            }
        }

        self.tick();
        Ok(())
    }

    /// Update the current time of the table
    pub fn tick(&mut self) {
        self.spinner.update();
    }

    /// Select the next row
    pub fn next(&mut self) {
        let num_rows = self.len();
        let i = match self.scroll.selected() {
            Some(i) => (i + 1).clamp(0, num_rows - 1),
            None => 0,
        };
        self.scroll.select(Some(i));
    }

    /// Select the previous row
    pub fn previous(&mut self) {
        let i = match self.scroll.selected() {
            Some(0) => 0,
            Some(i) => i - 1,
            None => 0,
        };
        self.scroll.select(Some(i));
    }

    pub fn get(&self, i: usize) -> Option<&str> {
        if i < self.finished.len() {
            let task = self.finished.get(i)?;
            Some(task.name())
        } else if i < self.finished.len() + self.running.len() {
            let task = self.running.get(i - self.finished.len())?;
            Some(task.name())
        } else if i < self.finished.len() + self.running.len() + self.planned.len() {
            let task = self
                .planned
                .get(i - (self.finished.len() + self.running.len()))?;
            Some(task.name())
        } else {
            None
        }
    }

    pub fn selected(&self) -> Option<&str> {
        let i = self.scroll.selected()?;
        self.get(i)
    }

    pub fn tasks_started(&self) -> impl Iterator<Item = &str> + '_ {
        self.finished
            .iter()
            .map(|task| task.name())
            .chain(self.running.iter().map(|task| task.name()))
    }

    fn finished_rows(&self) -> impl Iterator<Item = Row> + '_ {
        self.finished.iter().map(move |task| {
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
        self.running
            .iter()
            .map(move |task| Row::new(vec![Cell::new(task.name()), Cell::new(Text::raw(spinner))]))
    }

    fn planned_rows(&self) -> impl Iterator<Item = Row> + '_ {
        self.planned
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
        let bar = "─".repeat(usize::from(width));
        let table = Table::new(
            self.finished_rows()
                .chain(self.running_rows())
                .chain(self.planned_rows()),
            [
                Constraint::Min(4),
                // Status takes one cell to render
                Constraint::Length(1),
            ],
        )
        .highlight_style(Style::default().fg(Color::Yellow))
        .column_spacing(0)
        .header(
            vec![format!("Task\n{bar}"), "\n─".to_owned()]
                .into_iter()
                .map(Cell::from)
                .collect::<Row>()
                .height(2),
        );
        StatefulWidget::render(table, area, buf, state);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_scroll() {
        let mut table = TaskTable::new(vec![
            "foo".to_string(),
            "bar".to_string(),
            "baz".to_string(),
        ]);
        assert_eq!(table.scroll.selected(), None, "starts with no selection");
        table.next();
        assert_eq!(table.scroll.selected(), Some(0), "scroll starts from 0");
        table.previous();
        assert_eq!(table.scroll.selected(), Some(0), "scroll stays in bounds");
        table.next();
        table.next();
        assert_eq!(table.scroll.selected(), Some(2), "scroll moves forwards");
        table.next();
        assert_eq!(table.scroll.selected(), Some(2), "scroll stays in bounds");
    }

    #[test]
    fn test_selection_follows() {
        let mut table = TaskTable::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        table.next();
        table.next();
        assert_eq!(table.scroll.selected(), Some(1), "selected b");
        assert_eq!(table.selected(), Some("b"), "selected b");
        table.start_task("b").unwrap();
        assert_eq!(table.scroll.selected(), Some(0), "b stays selected");
        assert_eq!(table.selected(), Some("b"), "selected b");
        table.start_task("a").unwrap();
        assert_eq!(table.scroll.selected(), Some(0), "b stays selected");
        assert_eq!(table.selected(), Some("b"), "selected b");
        table.finish_task("a", TaskResult::Success).unwrap();
        assert_eq!(table.scroll.selected(), Some(1), "b stays selected");
        assert_eq!(table.selected(), Some("b"), "selected b");
    }

    #[test]
    fn test_restart_task() {
        let mut table = TaskTable::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        table.next();
        table.next();
        // Start all tasks
        table.start_task("b").unwrap();
        table.start_task("a").unwrap();
        table.start_task("c").unwrap();
        assert_eq!(table.get(0), Some("b"), "b is on top (running)");
        table.finish_task("a", TaskResult::Success).unwrap();
        assert_eq!(
            (table.get(0), table.get(1)),
            (Some("a"), Some("b")),
            "a is on top (done), b is second (running)"
        );

        table.finish_task("b", TaskResult::Success).unwrap();
        assert_eq!(
            (table.get(0), table.get(1)),
            (Some("a"), Some("b")),
            "a is on top (done), b is second (done)"
        );

        // Restart b
        table.start_task("b").unwrap();
        assert_eq!(
            (table.get(1), table.get(2)),
            (Some("c"), Some("b")),
            "b is third (running)"
        );

        // Restart a
        table.start_task("a").unwrap();
        assert_eq!(
            (table.get(0), table.get(1), table.get(2)),
            (Some("c"), Some("b"), Some("a")),
            "c is on top (running), b is second (running), a is third (running)"
        );
    }

    #[test]
    fn test_selection_stable() {
        let mut table = TaskTable::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        table.next();
        table.next();
        assert_eq!(table.scroll.selected(), Some(1), "selected b");
        assert_eq!(table.selected(), Some("b"), "selected b");
        // start c which moves it to "running" which is before "planned"
        table.start_task("c").unwrap();
        assert_eq!(table.scroll.selected(), Some(2), "selection stays on b");
        assert_eq!(table.selected(), Some("b"), "selected b");
        table.start_task("a").unwrap();
        assert_eq!(table.scroll.selected(), Some(2), "selection stays on b");
        assert_eq!(table.selected(), Some("b"), "selected b");
        // c
        // a
        // b <-
        table.previous();
        table.previous();
        assert_eq!(table.scroll.selected(), Some(0), "selected c");
        assert_eq!(table.selected(), Some("c"), "selected c");
        table.finish_task("a", TaskResult::Success).unwrap();
        assert_eq!(table.scroll.selected(), Some(1), "c stays selected");
        assert_eq!(table.selected(), Some("c"), "selected c");
        table.previous();
        table.finish_task("c", TaskResult::Success).unwrap();
        assert_eq!(table.scroll.selected(), Some(0), "a stays selected");
        assert_eq!(table.selected(), Some("a"), "selected a");
    }
}
