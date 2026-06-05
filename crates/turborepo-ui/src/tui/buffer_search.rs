use turborepo_vt100 as vt100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferMatch {
    pub row: usize,
    pub col: u16,
}

#[derive(Debug, Clone)]
pub struct BufferSearchResults {
    query: String,
    matches: Vec<BufferMatch>,
    current: usize,
}

impl BufferSearchResults {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            matches: Vec::new(),
            current: 0,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn matches(&self) -> &[BufferMatch] {
        &self.matches
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn has_matches(&self) -> bool {
        !self.matches.is_empty()
    }

    pub fn modify_query(&mut self, parser: &vt100::Parser, modification: impl FnOnce(&mut String)) {
        modification(&mut self.query);
        self.current = 0;
        self.update_matches(parser);
    }

    pub fn refresh(&mut self, parser: &vt100::Parser) {
        self.update_matches(parser);
        if !self.matches.is_empty() {
            self.current = self.current.min(self.matches.len() - 1);
        } else {
            self.current = 0;
        }
    }

    fn update_matches(&mut self, parser: &vt100::Parser) {
        self.matches.clear();
        if self.query.is_empty() {
            return;
        }
        self.matches = find_matches(parser, &self.query);
    }

    pub fn next_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current = (self.current + 1) % self.matches.len();
    }

    pub fn previous_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current = self.current.checked_sub(1).unwrap_or(self.matches.len() - 1);
    }
}

fn lines_equal(a: &str, b: &str) -> bool {
    a.trim_end() == b.trim_end()
}

fn read_entire_row(entire: &vt100::EntireScreen<'_>, row: usize) -> String {
    let (_, cols) = entire.size();
    let mut line = String::new();
    for col in 0..cols {
        if let Some(cell) = entire.cell(row as u16, col) {
            line.push_str(cell.contents());
        }
    }
    line
}

fn read_screen_row(parser: &vt100::Parser, row: u16) -> String {
    let (_, cols) = parser.screen().size();
    let mut line = String::new();
    for col in 0..cols {
        if let Some(cell) = parser.screen().cell(row, col) {
            line.push_str(cell.contents());
        }
    }
    line
}

fn matches_at_cell_index(row_cells: &[&str], start_col: usize, query_lower: &str) -> bool {
    let mut query_chars = query_lower.chars();
    let Some(first_query_char) = query_chars.next() else {
        return true;
    };

    if start_col >= row_cells.len() {
        return false;
    }
    let first_cell = row_cells[start_col];
    let mut first_cell_chars = first_cell.chars();
    let Some(first_cell_char) = first_cell_chars.next() else {
        return false;
    };
    if first_cell_char.to_lowercase().next() != Some(first_query_char) {
        return false;
    }

    let query_len_chars = query_lower.chars().count();
    let mut text = String::with_capacity(query_lower.len());
    text.push_str(first_cell);
    let mut col = start_col + 1;
    while col < row_cells.len() && text.chars().count() < query_len_chars {
        text.push_str(row_cells[col]);
        col += 1;
    }

    text.chars().count() >= query_len_chars
        && text.to_lowercase().starts_with(query_lower)
}

fn matches_at_screen_col(screen: &vt100::Screen, row: u16, col: u16, query_lower: &str, cols: u16) -> bool {
    let query_chars = query_lower.chars().count();
    let mut text = String::new();
    let mut c = col;
    while c < cols && text.chars().count() < query_chars {
        if let Some(cell) = screen.cell(row, c) {
            text.push_str(cell.contents());
        }
        c += 1;
    }

    text.chars().count() >= query_chars
        && text.to_lowercase().starts_with(query_lower)
}

fn match_end_col(screen: &vt100::Screen, visible_row: u16, start_col: u16, query_lower: &str, cols: u16) -> Option<u16> {
    let query_chars = query_lower.chars().count();
    let mut text = String::new();
    let mut col = start_col;
    while col < cols && text.chars().count() < query_chars {
        if let Some(cell) = screen.cell(visible_row, col) {
            text.push_str(cell.contents());
        }
        col += 1;
    }

    if text.chars().count() >= query_chars
        && text.to_lowercase().starts_with(query_lower)
    {
        Some(col.saturating_sub(1))
    } else {
        None
    }
}

fn find_matches(parser: &vt100::Parser, query: &str) -> Vec<BufferMatch> {
    let entire = parser.entire_screen();
    let (total_rows, cols) = entire.size();
    let mut matches = Vec::new();
    let query_lower = query.to_lowercase();

    for row in 0..total_rows {
        let row_cells: Vec<&str> = (0..cols)
            .map(|col| {
                entire
                    .cell(row as u16, col)
                    .map(|cell| cell.contents())
                    .unwrap_or("")
            })
            .collect();

        for col in 0..cols {
            if matches_at_cell_index(&row_cells, col as usize, &query_lower) {
                matches.push(BufferMatch {
                    row,
                    col,
                });
            }
        }
    }

    matches
}

pub fn scroll_to_match(parser: &mut vt100::Parser, m: &BufferMatch, query: &str) {
    if query.is_empty() {
        parser.screen_mut().clear_selection();
        return;
    }

    let entire = parser.entire_screen();
    let expected_line = read_entire_row(&entire, m.row);
    let (_, cols) = entire.size();
    let entire_rows = entire.size().0;
    let mut best: Option<(usize, u16)> = None;
    let query_lower = query.to_lowercase();

    for offset in 0..=entire_rows {
        parser.screen_mut().set_scrollback(offset);
        let visible_rows = parser.screen().size().0;
        for v in 0..visible_rows {
            let line = read_screen_row(parser, v);
            if lines_equal(&line, &expected_line)
                && matches_at_screen_col(
                    parser.screen(),
                    v,
                    m.col,
                    &query_lower,
                    cols,
                )
            {
                best = Some((offset, v));
                break;
            }
        }
        if best.is_some() {
            break;
        }
    }

    let Some((offset, visible_row)) = best else {
        parser.screen_mut().clear_selection();
        return;
    };

    parser.screen_mut().set_scrollback(offset);
    let end_col = match_end_col(
        parser.screen(),
        visible_row,
        m.col,
        &query_lower,
        cols,
    )
    .unwrap_or(m.col);
    parser
        .screen_mut()
        .set_selection(visible_row, m.col, visible_row, end_col);
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_no_query_no_matches() {
        let parser = vt100::Parser::new(5, 20, 100);
        let mut results = BufferSearchResults::new();
        results.modify_query(&parser, |q| q.push_str("foo"));
        assert!(!results.has_matches());
    }

    #[test]
    fn test_finds_matches_in_scrollback() {
        let mut parser = vt100::Parser::new(2, 20, 100);
        parser.process(b"hello world\r\nfoo bar\r\n");
        let mut results = BufferSearchResults::new();
        results.modify_query(&parser, |q| q.push_str("foo"));
        assert!(results.has_matches());
        assert_eq!(results.matches()[0].row, 1);
    }

    #[test]
    fn test_next_match_wraps() {
        let mut parser = vt100::Parser::new(2, 20, 100);
        parser.process(b"aaa\r\n");
        let mut results = BufferSearchResults::new();
        results.modify_query(&parser, |q| q.push_str("a"));
        assert_eq!(results.matches().len(), 3);
        results.next_match();
        results.next_match();
        assert_eq!(results.current(), 2);
        results.next_match();
        assert_eq!(results.current(), 0);
    }

    #[test]
    fn test_scroll_to_match_shows_row_at_top() {
        let mut parser = vt100::Parser::new(2, 20, 100);
        parser.process(b"line1\r\nline2\r\nline3\r\nline4\r\n");
        let m = BufferMatch { row: 1, col: 0 };
        scroll_to_match(&mut parser, &m, "line2");
        assert_eq!(
            parser.screen().selected_text().as_deref(),
            Some("line2")
        );
        assert_eq!(parser.screen().cell(0, 0).unwrap().contents(), "l");
    }

    #[test]
    fn test_scroll_to_match_selects_correct_line_when_scrolled() {
        let mut parser = vt100::Parser::new(5, 40, 100);
        for i in 0..20 {
            parser.process(format!("line{i}\r\n").as_bytes());
        }
        let mut results = BufferSearchResults::new();
        results.modify_query(&parser, |q| q.push_str("line15"));
        let m = results.matches()[0];
        scroll_to_match(&mut parser, &m, "line15");
        assert_eq!(
            parser.screen().selected_text().as_deref(),
            Some("line15")
        );
    }

    #[test]
    fn test_scroll_to_match_selects_correct_line_with_duplicate_column() {
        let mut parser = vt100::Parser::new(5, 40, 100);
        parser.process(b"error alpha\r\n");
        for i in 0..10 {
            parser.process(format!("line{i}\r\n").as_bytes());
        }
        parser.process(b"error beta\r\n");
        let mut results = BufferSearchResults::new();
        results.modify_query(&parser, |q| q.push_str("error"));
        let beta_match = results
            .matches()
            .iter()
            .find(|m| read_entire_row(&parser.entire_screen(), m.row).contains("beta"))
            .copied()
            .expect("beta match");
        scroll_to_match(&mut parser, &beta_match, "error");
        assert_eq!(
            parser.screen().selected_text().as_deref(),
            Some("error")
        );
    }

    #[test]
    fn test_match_after_wide_character_prefix() {
        let mut parser = vt100::Parser::new(3, 40, 100);
        parser.process(" \x1b[32m✓\x1b[0m build done\r\n".as_bytes());
        let mut results = BufferSearchResults::new();
        results.modify_query(&parser, |q| q.push_str("build"));
        assert!(results.has_matches());
        let m = results.matches()[0];
        scroll_to_match(&mut parser, &m, "build");
        assert_eq!(
            parser.screen().selected_text().as_deref(),
            Some("build")
        );
        assert_eq!(m.col, 3);
    }

    #[test]
    fn test_scroll_to_match_after_resize() {
        let mut parser = vt100::Parser::new(5, 80, 100);
        for i in 0..20 {
            parser.process(format!("line{i} padding\r\n").as_bytes());
        }
        let output = parser.entire_screen().contents().into_bytes();
        let scrollback = parser.screen().scrollback();
        let mut parser = vt100::Parser::new(5, 40, 100);
        parser.process(&output);
        parser.screen_mut().set_scrollback(scrollback);

        let mut results = BufferSearchResults::new();
        results.modify_query(&parser, |q| q.push_str("line15"));
        let m = results.matches()[0];
        scroll_to_match(&mut parser, &m, "line15");
        assert!(
            parser
                .screen()
                .selected_text()
                .is_some_and(|text| text.to_lowercase().contains("line15"))
        );
    }

    #[test]
    fn test_case_insensitive_match() {
        let mut parser = vt100::Parser::new(5, 20, 100);
        parser.process(b"Hello ERROR world\r\n");
        let mut results = BufferSearchResults::new();
        results.modify_query(&parser, |q| q.push_str("error"));
        assert!(results.has_matches());
        scroll_to_match(&mut parser, &results.matches()[0], "error");
        assert_eq!(
            parser.screen().selected_text().as_deref(),
            Some("ERROR")
        );
    }
}
