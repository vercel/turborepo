use std::time::Instant;

use ratatui::text::Text;

#[derive(Debug, Clone, Copy)]
pub struct TaskDuration {
    width: u16,
    run_start: Instant,
    current: Instant,
    task_start: Instant,
    task_end: Option<Instant>,
}

impl TaskDuration {
    pub fn new(
        width: u16,
        run_start: Instant,
        current: Instant,
        task_start: Instant,
        task_end: Option<Instant>,
    ) -> Self {
        Self {
            width,
            run_start,
            current,
            task_start,
            task_end,
        }
    }

    fn run_duration_ms(&self) -> u128 {
        self.current.duration_since(self.run_start).as_millis()
    }

    fn start_offset(&self) -> u128 {
        self.task_start.duration_since(self.run_start).as_millis()
    }

    fn end_offset(&self) -> u128 {
        self.task_end
            .unwrap_or(self.current)
            .duration_since(self.run_start)
            .as_millis()
    }

    fn end_marker(&self) -> char {
        match self.task_end.is_some() {
            true => '|',
            false => '>',
        }
    }

    fn scale(&self) -> f64 {
        self.width as f64 / self.run_duration_ms() as f64
    }
}

impl From<TaskDuration> for Text<'static> {
    fn from(value: TaskDuration) -> Self {
        let scale = value.scale();
        let last_index = value.width - 1;
        // We clamp these to the last visible char in the case either of events happen
        // to be happen at the 'current' instant.
        let start_index = ((value.start_offset() as f64 * scale) as u16).min(last_index);
        let end_index = ((value.end_offset() as f64 * scale) as u16).min(last_index);

        let mut bar = String::with_capacity(value.width.into());
        for idx in 0..value.width {
            if idx < start_index {
                bar.push(' ');
            } else if idx == start_index {
                bar.push('|');
            } else if start_index < idx && idx < end_index {
                bar.push('-');
            } else if idx == end_index {
                bar.push(value.end_marker());
            } else {
                bar.push(' ');
            }
        }
        Text::raw(bar)
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
    use test_case::test_case;

    use super::*;

    #[test_case("|-------->", 0, None ; "full bar")]
    #[test_case("  |--|    ", 2, Some(5) ; "finished task")]
    #[test_case("     |--->", 5, None ; "late unfinished")]
    #[test_case("     |    ", 5, Some(5) ; "short task")]
    #[test_case("     ||   ", 5, Some(6) ; "no inner")]
    #[test_case("         |", 10, None ; "just started")]
    fn task_duration_render(expected: &str, start: u64, end: Option<u64>) {
        let width = 10;
        let run_start = Instant::now();
        let current = run_start + Duration::from_secs(10);
        let task_start = run_start + Duration::from_secs(start);
        let task_end = end.map(|end| run_start + Duration::from_secs(end));
        let duration = TaskDuration {
            width,
            run_start,
            current,
            task_start,
            task_end,
        };
        let text = Text::from(duration);
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, 1));
        text.render(buffer.area, &mut buffer);
        assert_eq!(buffer, Buffer::with_lines(vec![expected]));
    }
}
