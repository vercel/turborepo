use std::{
    io::{self, Stdout},
    time::{Duration, Instant},
};

use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Layout},
    text::Text,
    widgets::Widget,
    Frame, Terminal,
};

const HEIGHT: u16 = 60;
const PANE_HEIGHT: u16 = 40;
const FRAMERATE: Duration = Duration::from_millis(3);

use super::{input, AppReceiver, Error, Event, TaskTable, TerminalPane};

pub struct App<I> {
    table: TaskTable,
    pane: TerminalPane<I>,
    done: bool,
}

impl<I> App<I> {
    pub fn new(rows: u16, cols: u16, tasks: Vec<String>) -> Self {
        let mut this = Self {
            table: TaskTable::new(tasks.clone()),
            pane: TerminalPane::new(rows, cols, tasks),
            done: false,
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
}

/// Handle the rendering of the `App` widget based on events received by
/// `receiver`
pub fn run_app(tasks: Vec<String>, receiver: AppReceiver) -> Result<(), Error> {
    let mut terminal = startup()?;
    let size = terminal.size()?;

    let app: App<()> = App::new(PANE_HEIGHT, size.width, tasks);

    let result = run_app_inner(&mut terminal, app, receiver);

    cleanup(terminal)?;

    result
}

// Break out inner loop so we can use `?` without worrying about cleaning up the
// terminal.
fn run_app_inner<I, B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App<I>,
    receiver: AppReceiver,
) -> Result<(), Error> {
    // Render initial state to paint the screen
    terminal.draw(|f| view(&mut app, f))?;
    let mut last_render = Instant::now();

    while let Some(event) = poll(&receiver, last_render + FRAMERATE) {
        if let Some(message) = update(&mut app, event)? {
            // TODO: use term emulator to properly render this, blocked by PR #7713
            terminal.insert_before(1, |buf| {
                Text::raw(String::from_utf8_lossy(&message)).render(buf.area, buf)
            })?;
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
fn poll(receiver: &AppReceiver, deadline: Instant) -> Option<Event> {
    match input() {
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

fn update<I>(app: &mut App<I>, event: Event) -> Result<Option<Vec<u8>>, Error> {
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
    }
    Ok(None)
}

fn view<I>(app: &mut App<I>, f: &mut Frame) {
    let vertical = Layout::vertical([Constraint::Min(5), Constraint::Length(PANE_HEIGHT)]);
    let [table, pane] = vertical.areas(f.size());
    app.table.stateful_render(f, table);
    f.render_widget(&app.pane, pane);
}
