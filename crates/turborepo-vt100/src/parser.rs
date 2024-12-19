/// A parser for terminal output which produces an in-memory representation of
/// the terminal contents.
pub struct Parser {
    parser: vte::Parser,
    screen: crate::perform::WrappedScreen,
}

impl Parser {
    /// Creates a new terminal parser of the given size and with the given
    /// amount of scrollback.
    #[must_use]
    pub fn new(rows: u16, cols: u16, scrollback_len: usize) -> Self {
        Self {
            parser: vte::Parser::new(),
            screen: crate::perform::WrappedScreen(crate::Screen::new(
                crate::grid::Size { rows, cols },
                scrollback_len,
            )),
        }
    }

    /// Processes the contents of the given byte string, and updates the
    /// in-memory terminal state.
    pub fn process(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.parser.advance(&mut self.screen, *byte);
        }
    }

    /// Processes the contents of the given byte string, and updates the
    /// in-memory terminal state. Calls methods on the given `Callbacks`
    /// object when relevant escape sequences are seen.
    pub fn process_cb(
        &mut self,
        bytes: &[u8],
        callbacks: &mut impl crate::callbacks::Callbacks,
    ) {
        let mut screen = crate::perform::WrappedScreenWithCallbacks::new(
            &mut self.screen,
            callbacks,
        );
        for byte in bytes {
            self.parser.advance(&mut screen, *byte);
        }
    }

    /// Returns a reference to a `Screen` object containing the terminal
    /// state.
    #[must_use]
    pub fn screen(&self) -> &crate::Screen {
        &self.screen.0
    }

    /// Returns a mutable reference to a `Screen` object containing the
    /// terminal state.
    #[must_use]
    pub fn screen_mut(&mut self) -> &mut crate::Screen {
        &mut self.screen.0
    }

    /// Returns a reference to an `EntireScreen` object containing the
    /// terminal state where all contents including scrollback and displayed.
    #[must_use]
    pub fn entire_screen(&self) -> crate::EntireScreen {
        crate::EntireScreen::new(self.screen())
    }
}

impl Default for Parser {
    /// Returns a parser with dimensions 80x24 and no scrollback.
    fn default() -> Self {
        Self::new(24, 80, 0)
    }
}

impl std::io::Write for Parser {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.process(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
