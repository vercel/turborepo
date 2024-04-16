pub struct EntireScreen<'a> {
    screen: &'a crate::Screen,
    // If present, screen will be truncated to only display lines that fit in those cells
    max_lines: Option<usize>,
    size: (usize, u16),
}

impl<'a> EntireScreen<'a> {
    #[must_use]
    pub fn new(screen: &'a crate::Screen) -> Self {
        Self {
            size: screen.grid().size_with_contents(),
            screen,
            max_lines: None,
        }
    }

    pub fn with_max_lines(&mut self, max_lines: Option<usize>) {
        self.max_lines = max_lines;
    }

    #[must_use]
    pub fn cell(&self, row: u16, col: u16) -> Option<&crate::Cell> {
        match self.max_lines {
            // We need to do some trimming
            Some(max_lines) if self.size().0 > max_lines => {
                // in this case we fuck ourselves :) HARD
                let (height, _) = self.size();
                // Skip over these
                let lines_to_cut = (height - max_lines) as u16;
                self.screen
                    .grid()
                    .all_row(lines_to_cut + row)
                    .and_then(|r| r.get(col))
            }
            _ => self.screen.grid().all_row(row).and_then(|r| r.get(col)),
        }
    }

    #[must_use]
    pub fn contents(&self) -> String {
        let mut s = String::new();
        self.screen.grid().write_full_contents(&mut s);
        s
    }

    /// Size required to render all contents
    #[must_use]
    pub fn size(&self) -> (usize, u16) {
        self.size
    }
}
