use std::{
    io::{self, Stdout, Write},
    sync::mpsc,
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

use super::{input, AppReceiver, Error, Event, InputOptions, TaskTable, TerminalPane};
use crate::tui::task::Task;

pub enum LayoutSections {
    Pane,
    TaskList,
}

pub struct App<I> {
    table: TaskTable,
    pane: TerminalPane<I>,
    done: bool,
    input_options: InputOptions,
    started_tasks: Vec<String>,
    task_list: Vec<String>,
    selected_task_index: usize,
    layout_focus: LayoutSections,
}

pub enum Direction {
    Up,
    Down,
}

impl<I> App<I> {
    pub fn new(rows: u16, cols: u16, tasks: Vec<String>) -> Self {
        debug!("tasks: {tasks:?}");

        let num_of_tasks = tasks.len();
        // TODO: Probably can manage this better?
        let task_list = tasks.clone();

        let mut planned_task_list = tasks.into_iter().map(Task::new).collect::<Vec<_>>();
        planned_task_list.sort_unstable();
        planned_task_list.dedup();

        Self {
            table: TaskTable::new(planned_task_list.clone()),
            pane: TerminalPane::new(rows, cols, tasks),
            done: false,
            input_options: InputOptions {
                interact: false,
                // Check if stdin is a tty that we should read input from
                tty_stdin: atty::is(atty::Stream::Stdin),
            },
            started_tasks: Vec::with_capacity(num_of_tasks),
            task_list,
            selected_task_index: 0,
            layout_focus: LayoutSections::Pane,
        }
    }

    pub fn next(&mut self) {
        let num_rows = self.table.len();
        let i = match self.table.scroll.selected() {
            Some(i) => (i + 1).clamp(0, num_rows - 1),
            None => 0,
        };
        self.table.scroll.select(Some(i));
        let task = self.task_list[i].as_str();
        self.pane.select(task).unwrap();
    }

    pub fn previous(&mut self) {
        let i = match self.selected_task_index {
            0 => 0,
            i => i - 1,
        };
        self.table.scroll.select(Some(i));
        let task = self.task_list[i].as_str();
        self.pane.select(task).unwrap();
    }

    pub fn interact(&mut self, interact: bool) {
        let Some(selected_task) = self.table.selected() else {
            return;
        };
        if self.pane.has_stdin(selected_task) {
            self.input_options.interact = interact;
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
        if !self.input_options.interact {
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

    let (result, callback) = match run_app_inner(&mut terminal, &mut app, receiver) {
        Ok(callback) => (Ok(()), callback),
        Err(err) => (Err(err), None),
    };

    cleanup(terminal, app, callback)?;

    result
}

// Break out inner loop so we can use `?` without worrying about cleaning up the
// terminal.
fn run_app_inner<B: Backend + std::io::Write>(
    terminal: &mut Terminal<B>,
    app: &mut App<Box<dyn io::Write + Send>>,
    receiver: AppReceiver,
) -> Result<Option<mpsc::SyncSender<()>>, Error> {
    // Render initial state to paint the screen
    terminal.draw(|f| view(app, f))?;
    let mut last_render = Instant::now();
    let mut callback = None;
    while let Some(event) = poll(app.input_options, &receiver, last_render + FRAMERATE) {
        callback = update(app, event)?;
        if app.done {
            break;
        }
        if FRAMERATE <= last_render.elapsed() {
            terminal.draw(|f| view(app, f))?;
            last_render = Instant::now();
        }
    }

    Ok(callback)
}

/// Blocking poll for events, will only return None if app handle has been
/// dropped
fn poll(input_options: InputOptions, receiver: &AppReceiver, deadline: Instant) -> Option<Event> {
    match input(input_options) {
        Ok(Some(event)) => Some(event),
        Ok(None) => receiver.recv(deadline).ok(),
        // Unable to read from stdin, shut down and attempt to clean up
        Err(_) => Some(Event::InternalStop),
    }
}

const MIN_HEIGHT: u16 = 10;
const MIN_WIDTH: u16 = 20;

pub fn terminal_big_enough() -> Result<bool, Error> {
    let (width, height) = crossterm::terminal::size()?;
    Ok(width >= MIN_WIDTH && height >= MIN_HEIGHT)
}

/// Configures terminal for rendering App
fn startup() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Ensure all pending writes are flushed before we switch to alternative screen
    stdout.flush()?;
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
    callback: Option<mpsc::SyncSender<()>>,
) -> io::Result<()> {
    terminal.clear()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen,
    )?;
    let started_tasks = app.table.tasks_started();
    app.pane.persist_tasks(&started_tasks)?;
    crossterm::terminal::disable_raw_mode()?;
    terminal.show_cursor()?;
    // We can close the channel now that terminal is back restored to a normal state
    drop(callback);
    Ok(())
}

fn update(
    app: &mut App<Box<dyn io::Write + Send>>,
    event: Event,
) -> Result<Option<mpsc::SyncSender<()>>, Error> {
    match event {
        Event::StartTask { task } => {
            app.table.start_task(&task)?;
            app.started_tasks.push(task);
        }
        Event::TaskOutput { task, output } => {
            app.pane.process_output(&task, &output)?;
        }
        Event::Status { task, status } => {
            app.pane.set_status(&task, status)?;
        }
        Event::InternalStop => {
            app.done = true;
        }
        Event::Stop(callback) => {
            app.done = true;
            return Ok(Some(callback));
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
