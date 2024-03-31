pub struct EntireScreen<'a>(pub(crate) &'a crate::Screen);

impl<'a> EntireScreen<'a> {
    #[must_use]
    pub fn cell(&self, row: u16, col: u16) -> Option<&crate::Cell> {
        self.0.grid().all_row(row).and_then(|r| r.get(col))
    }

    #[must_use]
    pub fn contents(&self) -> String {
        let mut s = String::new();
        self.0.grid().write_full_contents(&mut s);
        s
    }

    /// Size required to render all contents
    #[must_use]
    pub fn size(&self) -> (usize, u16) {
        self.0.grid().size_with_contents()
    }
}
