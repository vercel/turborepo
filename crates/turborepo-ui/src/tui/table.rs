use std::time::Instant;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{
        Block, BorderType, Borders, Cell, Paragraph, Row, StatefulWidget, Table, TableState, Widget,
    },
};

use super::{
    task::{Finished, Planned, Running, Task},
    task_duration::TaskDuration,
};

const FOOTER_TEXT: &str = "Use arrow keys to navigate";

/// A widget that renders a table of their tasks and their current status
///
/// The table contains finished tasks, running tasks, and planned tasks rendered
/// in that order.
pub struct TaskTable {
    // Start of the run and the current time
    start: Instant,
    current: Instant,
    // Tasks to be displayed
    // Ordered by when they finished
    finished: Vec<Task<Finished>>,
    // Ordered by when they started
    running: Vec<Task<Running>>,
    // Ordered by task name
    planned: Vec<Task<Planned>>,
    // State used for showing things
    task_column_width: u16,
    scroll: TableState,
}

impl TaskTable {
    /// Construct a new table with all of the planned tasks
    pub fn new(tasks: impl IntoIterator<Item = String>) -> Self {
        let mut planned = tasks.into_iter().map(Task::new).collect::<Vec<_>>();
        planned.sort_unstable();
        planned.dedup();
        let task_column_width = planned
            .iter()
            .map(|task| task.name().len())
            .max()
            .unwrap_or_default()
            // Task column width should be large enough to fit "Task" title
            .max(4) as u16;
        Self {
            start: Instant::now(),
            current: Instant::now(),
            planned,
            running: Vec::new(),
            finished: Vec::new(),
            task_column_width,
            scroll: TableState::default(),
        }
    }

    /// Number of rows in the table
    pub fn len(&self) -> usize {
        self.finished.len() + self.running.len() + self.planned.len()
    }

    /// If there are no tasks in the table
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Mark the given planned task as started
    /// Errors if given task wasn't a planned task
    pub fn start_task(&mut self, task: &str) -> Result<(), &'static str> {
        let planned_idx = self
            .planned
            .binary_search_by(|planned_task| planned_task.name().cmp(task))
            .map_err(|_| "no task found")?;
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
        self.tick();
        Ok(())
    }

    /// Mark the given running task as finished
    /// Errors if given task wasn't a running task
    pub fn finish_task(&mut self, task: &str) -> Result<(), &'static str> {
        let running_idx = self
            .running
            .iter()
            .position(|running| running.name() == task)
            .ok_or("no task found")?;
        let old_row_idx = self.finished.len() + running_idx;
        let new_row_idx = self.finished.len();
        let running = self.running.remove(running_idx);
        self.finished.push(running.finish());

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
        self.current = Instant::now();
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

    fn finished_rows(&self, duration_width: u16) -> impl Iterator<Item = Row> + '_ {
        self.finished.iter().map(move |task| {
            Row::new(vec![
                Cell::new(task.name()),
                Cell::new(TaskDuration::new(
                    duration_width,
                    self.start,
                    self.current,
                    task.start(),
                    Some(task.end()),
                )),
            ])
        })
    }

    fn running_rows(&self, duration_width: u16) -> impl Iterator<Item = Row> + '_ {
        self.running.iter().map(move |task| {
            Row::new(vec![
                Cell::new(task.name()),
                Cell::new(TaskDuration::new(
                    duration_width,
                    self.start,
                    self.current,
                    task.start(),
                    None,
                )),
            ])
        })
    }

    fn planned_rows(&self, duration_width: u16) -> impl Iterator<Item = Row> + '_ {
        self.planned.iter().map(move |task| {
            Row::new(vec![
                Cell::new(task.name()),
                Cell::new(" ".repeat(duration_width as usize)),
            ])
        })
    }

    /// Convenience method which renders and updates scroll state
    pub fn stateful_render(&mut self, frame: &mut ratatui::Frame) {
        let mut scroll = self.scroll.clone();
        frame.render_stateful_widget(&*self, frame.size(), &mut scroll);
        self.scroll = scroll;
    }

    fn column_widths(&self, parent_width: u16) -> (u16, u16) {
        // We trim names to be 40 long (+1 for column divider)
        let name_col_width = 40.min(self.task_column_width) + 1;
        if name_col_width + 2 < parent_width {
            let status_width = parent_width - (name_col_width + 2);
            (name_col_width, status_width)
        } else {
            // If there isn't any space for the task status, just don't display anything
            (name_col_width, 0)
        }
    }

    fn render_footer(area: Rect, buf: &mut Buffer) {
        let footer = Paragraph::new(Line::from(FOOTER_TEXT)).centered().block(
            Block::default()
                .borders(Borders::TOP)
                .border_type(BorderType::Plain),
        );
        footer.render(area, buf);
    }
}

impl<'a> StatefulWidget for &'a TaskTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer, state: &mut Self::State) {
        let width = area.width;
        let (name_width, status_width) = self.column_widths(width);
        let areas = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([Constraint::Min(2), Constraint::Length(2)])
            .split(area);
        let table = Table::new(
            self.finished_rows(status_width)
                .chain(self.running_rows(status_width))
                .chain(self.planned_rows(status_width)),
            [
                Constraint::Min(name_width),
                Constraint::Length(status_width),
            ],
        )
        .highlight_style(Style::default().fg(Color::Yellow))
        .header(
            ["Task\n----", "Status\n------"]
                .iter()
                .copied()
                .map(Cell::from)
                .collect::<Row>()
                .height(2),
        );
        StatefulWidget::render(table, areas[0], buf, state);
        TaskTable::render_footer(areas[1], buf);
    }
}

#[cfg(test)]
mod test {
    // Used by assert_buffer_eq
    #[allow(unused_imports)]
    use indoc::indoc;
    use ratatui::assert_buffer_eq;

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
        table.start_task("b").unwrap();
        assert_eq!(table.scroll.selected(), Some(0), "b stays selected");
        table.start_task("a").unwrap();
        assert_eq!(table.scroll.selected(), Some(0), "b stays selected");
        table.finish_task("a").unwrap();
        assert_eq!(table.scroll.selected(), Some(1), "b stays selected");
    }

    #[test]
    fn test_selection_stable() {
        let mut table = TaskTable::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        table.next();
        table.next();
        assert_eq!(table.scroll.selected(), Some(1), "selected b");
        // start c which moves it to "running" which is before "planned"
        table.start_task("c").unwrap();
        assert_eq!(table.scroll.selected(), Some(2), "selection stays on b");
        table.start_task("a").unwrap();
        assert_eq!(table.scroll.selected(), Some(2), "selection stays on b");
        // c
        // a
        // b <-
        table.previous();
        table.previous();
        assert_eq!(table.scroll.selected(), Some(0), "selected c");
        table.finish_task("a").unwrap();
        assert_eq!(table.scroll.selected(), Some(1), "c stays selected");
        table.previous();
        table.finish_task("c").unwrap();
        assert_eq!(table.scroll.selected(), Some(0), "a stays selected");
    }

    #[test]
    fn test_footer_always_rendered() {
        let table = TaskTable::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        let area = Rect::new(0, 0, 14, 5);
        let mut buffer = Buffer::empty(area);
        let mut scroll = table.scroll.clone();
        StatefulWidget::render(&table, area, &mut buffer, &mut scroll);
        assert_buffer_eq!(
            buffer,
            Buffer::with_lines(vec![
                "Task   Status ",
                "----   ------ ",
                "a             ",
                "──────────────",
                "Use arrow keys",
            ])
        )
    }
}
