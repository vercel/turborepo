use std::{
    io::{self, Stdout},
    time::{Duration, Instant},
};

use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Layout},
    Frame, Terminal,
};
use tracing::debug;

const PANE_SIZE_RATIO: f32 = 3.0 / 4.0;
const FRAMERATE: Duration = Duration::from_millis(3);

use super::{input, AppReceiver, Error, Event, TaskTable, TerminalPane};

pub struct App<I> {
    table: TaskTable,
    pane: TerminalPane<I>,
    done: bool,
    interact: bool,
}

pub enum Direction {
    Up,
    Down,
}

impl<I> App<I> {
    pub fn new(rows: u16, cols: u16, tasks: Vec<String>) -> Self {
        debug!("tasks: {tasks:?}");
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
        let Some(selected_task) = self.table.selected() else {
            return;
        };
        if self.pane.has_stdin(selected_task) {
            self.interact = interact;
            self.pane.highlight(interact);
        }
    }

    pub fn scroll(&mut self, direction: Direction) {
        let Some(selected_task) = self.table.selected() else {
            return;
        };
        self.pane
            .scroll(selected_task, direction)
            .expect("selected task should be in pane");
    }

    pub fn term_size(&self) -> (u16, u16) {
        self.pane.term_size()
    }

    pub fn update_tasks(&mut self, tasks: Vec<String>) {
        self.table = TaskTable::new(tasks.clone());
        self.next();
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

    // Figure out pane width?
    let task_width_hint = TaskTable::width_hint(tasks.iter().map(|s| s.as_str()));
    // Want to maximize pane width
    let ratio_pane_width = (f32::from(size.width) * PANE_SIZE_RATIO) as u16;
    let full_task_width = size.width.saturating_sub(task_width_hint);

    let mut app: App<Box<dyn io::Write + Send>> =
        App::new(size.height, full_task_width.max(ratio_pane_width), tasks);

    let result = run_app_inner(&mut terminal, &mut app, receiver);

    cleanup(terminal, app)?;

    result
}

// Break out inner loop so we can use `?` without worrying about cleaning up the
// terminal.
fn run_app_inner<B: Backend + std::io::Write>(
    terminal: &mut Terminal<B>,
    app: &mut App<Box<dyn io::Write + Send>>,
    receiver: AppReceiver,
) -> Result<(), Error> {
    // Render initial state to paint the screen
    terminal.draw(|f| view(app, f))?;
    let mut last_render = Instant::now();
    while let Some(event) = poll(app.interact, &receiver, last_render + FRAMERATE) {
        update(app, event)?;
        if app.done {
            break;
        }
        if FRAMERATE <= last_render.elapsed() {
            terminal.draw(|f| view(app, f))?;
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
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::event::EnableMouseCapture,
        crossterm::terminal::EnterAlternateScreen
    )?;
    let backend = CrosstermBackend::new(stdout);

    let mut terminal = Terminal::with_options(
        backend,
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Fullscreen,
        },
    )?;
    terminal.hide_cursor()?;

    Ok(terminal)
}

/// Restores terminal to expected state
fn cleanup<B: Backend + io::Write, I>(
    mut terminal: Terminal<B>,
    mut app: App<I>,
) -> io::Result<()> {
    terminal.clear()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen,
    )?;
    let started_tasks = app.table.tasks_started().collect();
    app.pane.render_remaining(started_tasks)?;
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
        Event::Status { task, status } => {
            app.pane.set_status(&task, status)?;
        }
        Event::Stop => {
            app.done = true;
        }
        Event::Tick => {
            app.table.tick();
        }
        Event::EndTask { task, result } => {
            app.table.finish_task(&task, result)?;
        }
        Event::Up => {
            app.previous();
        }
        Event::Down => {
            app.next();
        }
        Event::ScrollUp => {
            app.scroll(Direction::Up);
        }
        Event::ScrollDown => {
            app.scroll(Direction::Down);
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
        Event::UpdateTasks { tasks } => {
            app.update_tasks(tasks);
            app.table.tick();
        }
    }
    Ok(None)
}

fn view<I>(app: &mut App<I>, f: &mut Frame) {
    let (_, width) = app.term_size();
    let vertical = Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]);
    let [table, pane] = vertical.areas(f.size());
    app.table.stateful_render(f, table);
    f.render_widget(&app.pane, pane);
}
