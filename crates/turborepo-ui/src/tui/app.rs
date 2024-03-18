use std::{
    io::{self, Stdout},
    time::{Duration, Instant},
};

use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Layout},
    widgets::Widget,
    Frame, Terminal,
};
use tui_term::widget::PseudoTerminal;

const HEIGHT: u16 = 60;
const PANE_HEIGHT: u16 = 40;
const FRAMERATE: Duration = Duration::from_millis(3);

use super::{input, AppReceiver, Error, Event, TaskTable, TerminalPane};

pub struct App<I> {
    table: TaskTable,
    pane: TerminalPane<I>,
    done: bool,
    interact: bool,
}

impl<I> App<I> {
    pub fn new(rows: u16, cols: u16, tasks: Vec<String>) -> Self {
        let mut this = Self {
            table: TaskTable::new(tasks.clone()),
            pane: TerminalPane::new(rows, cols, tasks),
            done: false,
            interact: false,
        };
        // Start with first task selected
        this.next();
        this
    }

    pub fn next(&mut self) {
        self.table.next();
        if let Some(task) = self.table.selected() {
            self.pane.select(task).unwrap();
        }
    }

    pub fn previous(&mut self) {
        self.table.previous();
        if let Some(task) = self.table.selected() {
            self.pane.select(task).unwrap();
        }
    }

    pub fn interact(&mut self, interact: bool) {
        self.interact = interact;
        self.pane.highlight(interact);
    }
}

impl<I: std::io::Write> App<I> {
    pub fn forward_input(&mut self, bytes: &[u8]) -> Result<(), Error> {
        // If we aren't in interactive mode, ignore input
        if !self.interact {
            return Ok(());
        }
        let selected_task = self
            .table
            .selected()
            .expect("table should always have task selected");
        self.pane.process_input(selected_task, bytes)?;
        Ok(())
    }
}

/// Handle the rendering of the `App` widget based on events received by
/// `receiver`
pub fn run_app(tasks: Vec<String>, receiver: AppReceiver) -> Result<(), Error> {
    let mut terminal = startup()?;
    let size = terminal.size()?;

    let app: App<Box<dyn io::Write + Send>> = App::new(PANE_HEIGHT, size.width, tasks);

    let result = run_app_inner(&mut terminal, app, receiver);

    cleanup(terminal)?;

    result
}

// Break out inner loop so we can use `?` without worrying about cleaning up the
// terminal.
fn run_app_inner<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App<Box<dyn io::Write + Send>>,
    receiver: AppReceiver,
) -> Result<(), Error> {
    // Render initial state to paint the screen
    terminal.draw(|f| view(&mut app, f))?;
    let mut last_render = Instant::now();

    while let Some(event) = poll(app.interact, &receiver, last_render + FRAMERATE) {
        if let Some(message) = update(&mut app, event)? {
            persist_bytes(terminal, &message)?;
        }
        if app.done {
            break;
        }
        if FRAMERATE <= last_render.elapsed() {
            terminal.draw(|f| view(&mut app, f))?;
            last_render = Instant::now();
        }
    }

    Ok(())
}

/// Blocking poll for events, will only return None if app handle has been
/// dropped
fn poll(interact: bool, receiver: &AppReceiver, deadline: Instant) -> Option<Event> {
    match input(interact) {
        Ok(Some(event)) => Some(event),
        Ok(None) => receiver.recv(deadline).ok(),
        // Unable to read from stdin, shut down and attempt to clean up
        Err(_) => Some(Event::Stop),
    }
}

/// Configures terminal for rendering App
fn startup() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    crossterm::terminal::enable_raw_mode()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Inline(HEIGHT),
        },
    )?;
    terminal.hide_cursor()?;

    Ok(terminal)
}

/// Restores terminal to expected state
fn cleanup<B: Backend>(mut terminal: Terminal<B>) -> io::Result<()> {
    terminal.clear()?;
    crossterm::terminal::disable_raw_mode()?;
    terminal.show_cursor()?;
    Ok(())
}

fn update(
    app: &mut App<Box<dyn io::Write + Send>>,
    event: Event,
) -> Result<Option<Vec<u8>>, Error> {
    match event {
        Event::StartTask { task } => {
            app.table.start_task(&task)?;
        }
        Event::TaskOutput { task, output } => {
            app.pane.process_output(&task, &output)?;
        }
        Event::Stop => {
            app.done = true;
        }
        Event::Tick => {
            app.table.tick();
        }
        Event::Log { message } => {
            return Ok(Some(message));
        }
        Event::EndTask { task } => {
            app.table.finish_task(&task)?;
        }
        Event::Up => {
            app.previous();
        }
        Event::Down => {
            app.next();
        }
        Event::EnterInteractive => {
            app.interact(true);
        }
        Event::ExitInteractive => {
            app.interact(false);
        }
        Event::Input { bytes } => {
            app.forward_input(&bytes)?;
        }
        Event::SetStdin { task, stdin } => {
            app.pane.insert_stdin(&task, Some(stdin))?;
        }
    }
    Ok(None)
}

fn view<I>(app: &mut App<I>, f: &mut Frame) {
    let vertical = Layout::vertical([Constraint::Min(5), Constraint::Length(PANE_HEIGHT)]);
    let [table, pane] = vertical.areas(f.size());
    app.table.stateful_render(f, table);
    f.render_widget(&app.pane, pane);
}

/// Write provided bytes to a section of the screen that won't get rewritten
fn persist_bytes(terminal: &mut Terminal<impl Backend>, bytes: &[u8]) -> Result<(), Error> {
    let size = terminal.size()?;
    let mut parser = turborepo_vt100::Parser::new(size.height, size.width, 128);
    parser.process(bytes);
    let screen = parser.entire_screen();
    let (height, _) = screen.size();
    terminal.insert_before(height as u16, |buf| {
        PseudoTerminal::new(&screen).render(buf.area, buf)
    })?;
    Ok(())
}

#[cfg(test)]
mod test {
    use ratatui::{backend::TestBackend, buffer::Buffer};

    use super::*;

    #[test]
    fn test_persist_bytes() {
        let mut term = Terminal::with_options(
            TestBackend::new(10, 7),
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Inline(3),
            },
        )
        .unwrap();
        persist_bytes(&mut term, b"two\r\nlines").unwrap();
        term.backend().assert_buffer(&Buffer::with_lines(vec![
            "two       ",
            "lines     ",
            "          ",
            "          ",
            "          ",
            "          ",
            "          ",
        ]));
    }
}
