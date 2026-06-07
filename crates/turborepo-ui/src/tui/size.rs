use super::PANE_LEFT_PADDING_WITH_SIDEBAR;
use crate::TaskTable;

const PANE_SIZE_RATIO: f32 = 3.0 / 4.0;

#[derive(Debug, Clone, Copy)]
pub struct SizeInfo {
    task_width_hint: u16,
    rows: u16,
    cols: u16,
}

impl SizeInfo {
    pub fn new<'a>(rows: u16, cols: u16, tasks: impl Iterator<Item = &'a str>) -> Self {
        let task_width_hint = TaskTable::width_hint(tasks);
        Self {
            rows,
            cols,
            task_width_hint,
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
    }

    pub fn pane_rows(&self) -> u16 {
        self.rows
            // Account for header and footer in layout
            .saturating_sub(2)
            // Always allocate at least one row as vt100 crashes if emulating a zero area terminal
            .max(1)
    }

    pub fn task_list_width(&self) -> u16 {
        self.cols.saturating_sub(self.rendered_pane_cols())
    }

    pub fn pane_cols(&self) -> u16 {
        self.pane_cols_with_sidebar(true)
    }

    pub fn pane_cols_with_sidebar(&self, has_sidebar: bool) -> u16 {
        if has_sidebar {
            // Want to maximize pane width
            let ratio_pane_width = (f32::from(self.cols) * PANE_SIZE_RATIO) as u16;
            let full_task_width = self.cols.saturating_sub(self.task_width_hint);
            full_task_width
                .max(ratio_pane_width)
                // Account for the task list border and the spacer before pane content.
                .saturating_sub(1 + PANE_LEFT_PADDING_WITH_SIDEBAR)
        } else {
            // When sidebar is hidden, pane takes full width minus border
            self.cols.saturating_sub(1)
        }
    }

    pub fn rendered_pane_cols(&self) -> u16 {
        self.rendered_pane_cols_with_sidebar(true)
    }

    pub fn rendered_pane_cols_with_sidebar(&self, has_sidebar: bool) -> u16 {
        self.pane_cols_with_sidebar(has_sidebar)
            .saturating_add(if has_sidebar {
                PANE_LEFT_PADDING_WITH_SIDEBAR
            } else {
                0
            })
    }

    pub fn pane_left_padding_with_sidebar(&self, has_sidebar: bool) -> u16 {
        if has_sidebar {
            PANE_LEFT_PADDING_WITH_SIDEBAR
        } else {
            0
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_rendered_pane_includes_left_padding_when_sidebar_visible() {
        let size = SizeInfo::new(20, 100, ["app-a#dev"].into_iter());

        assert_eq!(size.rendered_pane_cols(), size.pane_cols() + 1);
        assert_eq!(size.task_list_width() + size.rendered_pane_cols(), 100);
    }

    #[test]
    fn test_rendered_pane_has_no_padding_when_sidebar_hidden() {
        let size = SizeInfo::new(20, 100, ["app-a#dev"].into_iter());

        assert_eq!(
            size.rendered_pane_cols_with_sidebar(false),
            size.pane_cols_with_sidebar(false)
        );
        assert_eq!(size.pane_left_padding_with_sidebar(false), 0);
    }
}
