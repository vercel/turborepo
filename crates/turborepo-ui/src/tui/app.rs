use std::{
    collections::BTreeMap,
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
use crate::tui::{
    task::{Task, TasksByStatus},
    term_output::TerminalOutput,
};

pub enum LayoutSections {
    Pane,
    TaskList,
}

pub struct App<W> {
    // TODO: Could I reasonably get to only one representation of the tasks?
    tasks: BTreeMap<String, TerminalOutput<W>>,
    tasks_by_status: TasksByStatus,
    input_options: InputOptions,
    scroll: TableState,
    selected_task_index: usize,
    has_user_scrolled: bool,
    done: bool,
}

pub enum Direction {
    Up,
    Down,
}

impl<W> App<W> {
    pub fn new(rows: u16, cols: u16, tasks: Vec<String>) -> Self {
        debug!("tasks: {tasks:?}");

        // Initializes with the planned tasks
        // and will mutate as tasks change
        // to running, finished, etc.
        let mut task_list = tasks.clone().into_iter().map(Task::new).collect::<Vec<_>>();
        task_list.sort_unstable();
        task_list.dedup();

        let tasks_by_status = TasksByStatus {
            planned: task_list,
            finished: Vec::new(),
            running: Vec::new(),
        };

        let has_user_interacted = false;
        let selected_task_index: usize = 0;

        Self {
            done: false,
            input_options: InputOptions {
                focus: LayoutSections::TaskList,
                // Check if stdin is a tty that we should read input from
                tty_stdin: atty::is(atty::Stream::Stdin),
            },
            tasks: tasks_by_status
                .task_names_in_displayed_order()
                .into_iter()
                .map(|task_name| (task_name, TerminalOutput::new(rows, cols, None)))
                .collect(),
            tasks_by_status,
            scroll: TableState::default().with_selected(selected_task_index),
            selected_task_index,
            has_user_scrolled: has_user_interacted,
        }
    }

    pub fn is_focusing_pane(&self) -> bool {
        match self.input_options.focus {
            LayoutSections::Pane => true,
            LayoutSections::TaskList => false,
        }
    }

    pub fn active_task(&self) -> String {
        self.tasks_by_status
            .task_names_in_displayed_order()
            .remove(self.selected_task_index)
    }

    pub fn get_full_task_mut(&mut self) -> &mut TerminalOutput<W> {
        self.tasks.get_mut(&self.active_task()).unwrap()
    }

    pub fn next(&mut self) {
        let num_rows = self.tasks_by_status.count_all();
        let next_index = (self.selected_task_index + 1).clamp(0, num_rows - 1);
        self.selected_task_index = next_index;
        self.scroll.select(Some(next_index));
        self.has_user_scrolled = true;
    }

    pub fn previous(&mut self) {
        let i = match self.selected_task_index {
            0 => 0,
            i => i - 1,
        };
        self.selected_task_index = i;
        self.scroll.select(Some(i));
        self.has_user_scrolled = true;
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
        if !self.has_user_scrolled {
            return Ok(());
        }

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
        if !self.has_user_scrolled {
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

    pub fn has_stdin(&self) -> bool {
        let active_task = self.active_task();
        if let Some(term) = self.tasks.get(&active_task) {
            term.stdin.is_some()
        } else {
            false
        }
    }

    pub fn interact(&mut self) {
        if matches!(self.input_options.focus, LayoutSections::Pane) {
            self.input_options.focus = LayoutSections::TaskList
        } else if self.has_stdin() {
            self.input_options.focus = LayoutSections::Pane;
        }
    }

    pub fn update_tasks(&mut self, tasks: Vec<String>) {
        let mut task_list = tasks.into_iter().map(Task::new).collect::<Vec<_>>();
        task_list.sort_unstable();
        task_list.dedup();
    }

    /// Persist all task output to the after closing the TUI
    pub fn persist_tasks(&mut self, started_tasks: Vec<String>) -> std::io::Result<()> {
        for (task_name, task) in started_tasks.into_iter().filter_map(|started_task| {
            (Some(started_task.clone())).zip(self.tasks.get(&started_task))
        }) {
            task.persist_screen(&task_name)?;
        }
        Ok(())
    }

    pub fn set_status(&mut self, status: String) -> Result<(), Error> {
        let task = self.get_full_task_mut();
        task.status = Some(status);
        Ok(())
    }
}

impl<W: Write> App<W> {
    /// Insert a stdin to be associated with a task
    pub fn insert_stdin(&mut self, stdin: Option<W>) -> Result<(), Error> {
        let task = self.get_full_task_mut();
        task.stdin = stdin;
        Ok(())
    }

    pub fn forward_input(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if matches!(self.input_options.focus, LayoutSections::Pane) {
            let task_output = self.get_full_task_mut();
            if let Some(stdin) = &mut task_output.stdin {
                stdin.write_all(bytes).map_err(|e| Error::Stdin {
                    name: self.active_task(),
                    e,
                })?;
            }
            Ok(())
        } else {
            Ok(())
        }
    }

    pub fn process_output(&mut self, task: &str, output: &[u8]) -> Result<(), Error> {
        let task_output = self.tasks.get_mut(task).unwrap();
        task_output.parser.process(output);
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

    let mut app: App<Box<dyn io::Write + Send>> = App::new(size.height, size.width, tasks);

    let (result, callback) = match run_app_inner(
        &mut terminal,
        &mut app,
        receiver,
        size.height,
        full_task_width.max(ratio_pane_width),
    ) {
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
    rows: u16,
    cols: u16,
) -> Result<Option<mpsc::SyncSender<()>>, Error> {
    // Render initial state to paint the screen
    terminal.draw(|f| view(app, f, rows, cols))?;
    let mut last_render = Instant::now();
    let mut callback = None;
    while let Some(event) = poll(&app.input_options, &receiver, last_render + FRAMERATE) {
        callback = update(app, event)?;
        if app.done {
            break;
        }
        if FRAMERATE <= last_render.elapsed() {
            terminal.draw(|f| view(app, f, rows, cols))?;
            last_render = Instant::now();
        }
    }

    Ok(callback)
}

/// Blocking poll for events, will only return None if app handle has been
/// dropped
fn poll(input_options: &InputOptions, receiver: &AppReceiver, deadline: Instant) -> Option<Event> {
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
fn cleanup<B: Backend + io::Write>(
    mut terminal: Terminal<B>,
    mut app: App<Box<dyn io::Write + Send>>,
    callback: Option<mpsc::SyncSender<()>>,
) -> io::Result<()> {
    terminal.clear()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen,
    )?;
    let tasks_started = app.tasks_by_status.tasks_started();
    app.persist_tasks(tasks_started)?;
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
            app.tasks_by_status
                .groups_as_task_names()
                .running
                .push(task);
        }
        Event::TaskOutput { task, output } => {
            app.process_output(&task, &output)?;
        }
        Event::Status { status } => {
            app.set_status(status)?;
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
            app.has_user_scrolled = true;
            app.tasks
                .get_mut(&app.active_task())
                .unwrap()
                .scroll(Direction::Up)
                .unwrap_or_default();
        }
        Event::ScrollDown => {
            app.has_user_scrolled = true;
            app.tasks
                .get_mut(&app.active_task())
                .unwrap()
                .scroll(Direction::Down)
                .unwrap_or_default();
        }
        Event::EnterInteractive => {
            app.has_user_scrolled = true;
            app.interact();
        }
        Event::ExitInteractive => {
            app.has_user_scrolled = true;
            app.interact();
        }
        Event::Input { bytes } => {
            app.forward_input(&bytes)?;
        }
        Event::SetStdin { stdin } => {
            app.insert_stdin(Some(stdin))?;
        }
        Event::UpdateTasks { tasks } => {
            app.update_tasks(tasks);
            // app.table.tick();
        }
    }
    Ok(None)
}

fn view(app: &mut App<Box<dyn io::Write + Send>>, f: &mut Frame, rows: u16, cols: u16) {
    let horizontal = Layout::horizontal([Constraint::Fill(1), Constraint::Length(cols)]);
    let [table, pane] = horizontal.areas(f.size());

    let active_task = app.active_task();

    let pane_to_render: TerminalPane<Box<dyn io::Write + Send>> = TerminalPane::new(
        rows,
        cols,
        app.is_focusing_pane(),
        &mut app.tasks,
        &active_task,
    );

    let table_to_render = TaskTable::new(&app.tasks_by_status);

    f.render_stateful_widget(&table_to_render, table, &mut app.scroll);
    f.render_widget(&pane_to_render, pane);
}
