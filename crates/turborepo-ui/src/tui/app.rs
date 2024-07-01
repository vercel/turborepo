use std::{
    io::{self, Stdout, Write},
    sync::mpsc,
    time::{Duration, Instant},
};

use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Layout},
    widgets::TableState,
    Frame, Terminal,
};
use tracing::debug;

const PANE_SIZE_RATIO: f32 = 3.0 / 4.0;
const FRAMERATE: Duration = Duration::from_millis(3);

use super::{
    event::TaskResult, input, AppReceiver, Error, Event, InputOptions, TaskTable, TerminalPane,
};
use crate::tui::task::{Task, TasksByStatus};

pub enum LayoutSections {
    Pane,
    TaskList,
}

pub struct App<I> {
    pane: TerminalPane<I>,
    done: bool,
    input_options: InputOptions,
    started_tasks: Vec<String>,
    task_list: Vec<String>,
    scroll: TableState,
    tasks_by_status: TasksByStatus,
    selected_task_index: usize,
    has_user_interacted: bool,
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

        // Initializes with the planned tasks
        // and will mutate as tasks change
        // to running, finished, etc.
        let mut task_list = tasks.clone().into_iter().map(Task::new).collect::<Vec<_>>();
        task_list.sort_unstable();
        task_list.dedup();

        // TODO: WIP, I shouldn't need this when I'm done?
        let task_list_as_strings = task_list
            .clone()
            .into_iter()
            .map(|task| task.name().to_string())
            .collect::<Vec<_>>();

        let tasks_by_status = TasksByStatus {
            planned: task_list,
            finished: Vec::new(),
            running: Vec::new(),
        };

        let has_user_interacted = false;
        let selected_task_index: usize = 0;

        Self {
            pane: TerminalPane::new(rows, cols, tasks),
            done: false,
            input_options: InputOptions {
                interact: false,
                // Check if stdin is a tty that we should read input from
                tty_stdin: atty::is(atty::Stream::Stdin),
            },
            started_tasks: Vec::with_capacity(num_of_tasks),
            task_list: task_list_as_strings,
            tasks_by_status,
            scroll: TableState::default().with_selected(selected_task_index),
            selected_task_index,
            has_user_interacted,
            layout_focus: LayoutSections::Pane,
        }
    }

    pub fn next(&mut self) {
        let num_rows = self.task_list.len();
        let next_index = (self.selected_task_index + 1).clamp(0, num_rows - 1);
        self.selected_task_index = next_index;
        self.scroll.select(Some(next_index));
        let task = self.task_list[next_index].as_str();
        self.pane.select(task).unwrap();
        self.has_user_interacted = true;
    }

    pub fn previous(&mut self) {
        let i = match self.selected_task_index {
            0 => 0,
            i => i - 1,
        };
        self.selected_task_index = i;
        self.scroll.select(Some(i));
        let task = self.task_list[i].as_str();
        self.pane.select(task).unwrap();
        self.has_user_interacted = true;
    }

    /// Mark the given task as started.
    /// If planned, pulls it from planned tasks and starts it.
    /// If finished, removes from finished and starts again as new task.
    pub fn start_task(&mut self, task: &str) -> Result<(), Error> {
        // Name of currently highlighted task.
        // We will use this after the order switches.
        let highlighted_task =
            &self.tasks_by_status.task_names_in_displayed_order()[self.selected_task_index];

        let planned_idx = self
            .tasks_by_status
            .planned
            .iter()
            .position(|planned| planned.name() == task)
            .ok_or_else(|| {
                debug!("could not find '{task}' to start");
                Error::TaskNotFound { name: task.into() }
            })?;

        let planned = self.tasks_by_status.planned.remove(planned_idx);
        let running = planned.start();
        self.tasks_by_status.running.push(running);

        // If user hasn't interacted, keep highlighting top-most task in list.
        if !self.has_user_interacted {
            return Ok(());
        }

        // Find the highlighted task from before the list movement in the new list.
        if let Some(new_index_to_highlight) = self
            .tasks_by_status
            .task_names_in_displayed_order()
            .iter()
            .position(|running| running == highlighted_task)
        {
            self.selected_task_index = new_index_to_highlight;
            self.scroll.select(Some(new_index_to_highlight));
        }

        Ok(())
    }

    /// Mark the given running task as finished
    /// Errors if given task wasn't a running task
    pub fn finish_task(&mut self, task: &str, result: TaskResult) -> Result<(), Error> {
        // Name of currently highlighted task.
        // We will use this after the order switches.
        let highlighted_task =
            &self.tasks_by_status.task_names_in_displayed_order()[self.selected_task_index];

        let running_idx = self
            .tasks_by_status
            .running
            .iter()
            .position(|running| running.name() == task)
            .ok_or_else(|| {
                debug!("could not find '{task}' to finish");
                println!("{:#?}", highlighted_task);
                Error::TaskNotFound { name: task.into() }
            })?;

        let running = self.tasks_by_status.running.remove(running_idx);
        self.tasks_by_status.finished.push(running.finish(result));

        // If user hasn't interacted, keep highlighting top-most task in list.
        if !self.has_user_interacted {
            return Ok(());
        }

        // Find the highlighted task from before the list movement in the new list.
        if let Some(new_index_to_highlight) = self
            .tasks_by_status
            .task_names_in_displayed_order()
            .iter()
            .position(|running| running == highlighted_task)
        {
            self.selected_task_index = new_index_to_highlight;
            self.scroll.select(Some(new_index_to_highlight));
        }

        Ok(())
    }

    pub fn interact(&mut self, interact: bool) {
        if self
            .pane
            .has_stdin(self.task_list[self.selected_task_index].as_str())
        {
            self.input_options.interact = interact;
            self.pane.highlight(interact);
        }
    }

    pub fn scroll(&mut self, direction: Direction) {
        self.pane
            .scroll(self.task_list[self.selected_task_index].as_str(), direction)
            .expect("selected task should be in pane");
    }

    pub fn term_size(&self) -> (u16, u16) {
        self.pane.term_size()
    }

    pub fn update_tasks(&mut self, tasks: Vec<String>) {
        let mut task_list = tasks.into_iter().map(Task::new).collect::<Vec<_>>();
        task_list.sort_unstable();
        task_list.dedup();

        self.next();
    }
}

impl<I: std::io::Write> App<I> {
    pub fn forward_input(&mut self, bytes: &[u8]) -> Result<(), Error> {
        // If we aren't in interactive mode, ignore input
        if !self.input_options.interact {
            return Ok(());
        }
        let selected_task = self.task_list[self.selected_task_index].as_str();
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
    app.pane
        .persist_tasks(&app.tasks_by_status.tasks_started())?;
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
            app.start_task(&task)?;
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
            // app.table.tick();
        }
        Event::EndTask { task, result } => {
            app.finish_task(&task, result)?;
        }
        Event::Up => {
            app.previous();
        }
        Event::Down => {
            app.next();
        }
        Event::ScrollUp => {
            app.has_user_interacted = true;
            app.scroll(Direction::Up);
        }
        Event::ScrollDown => {
            app.has_user_interacted = true;
            app.scroll(Direction::Down);
        }
        Event::EnterInteractive => {
            app.has_user_interacted = true;
            app.interact(true);
        }
        Event::ExitInteractive => {
            app.has_user_interacted = true;
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
            // app.table.tick();
        }
    }
    Ok(None)
}

fn view<I>(app: &mut App<I>, f: &mut Frame) {
    let (_, width) = app.term_size();
    let horizontal = Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]);
    let [table, pane] = horizontal.areas(f.size());

    let tabley_boi = TaskTable::new(
        &app.tasks_by_status,
        &app.selected_task_index,
        &app.has_user_interacted,
    );

    f.render_stateful_widget(&tabley_boi, table, &mut app.scroll);
    f.render_widget(&app.pane, pane);
}
