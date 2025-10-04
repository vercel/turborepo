use std::{
    collections::BTreeMap,
    io::{self, Stdout, Write},
    mem,
    time::Duration,
};

use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Layout},
    widgets::{Clear, TableState},
};
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};
use tracing::{debug, trace};
use turbopath::AbsoluteSystemPathBuf;

use crate::tui::popup::{popup, popup_area};

pub const FRAMERATE: Duration = Duration::from_millis(3);
const RESIZE_DEBOUNCE_DELAY: Duration = Duration::from_millis(10);

use super::{
    AppReceiver, Debouncer, Error, Event, InputOptions, SizeInfo, TaskTable, TerminalPane,
    event::{CacheResult, Direction, OutputLogs, PaneSize, TaskResult},
    input,
    preferences::PreferenceLoader,
    search::SearchResults,
};
use crate::{
    ColorConfig,
    tui::{
        scroll::ScrollMomentum,
        task::{Task, TasksByStatus},
        term_output::TerminalOutput,
    },
};

#[derive(Debug, Clone)]
pub enum LayoutSections {
    Pane,
    TaskList,
    Search {
        previous_selection: String,
        results: SearchResults,
    },
    SearchLocked {
        results: SearchResults,
    },
}

pub struct App<W> {
    size: SizeInfo,
    tasks: BTreeMap<String, TerminalOutput<W>>,
    tasks_by_status: TasksByStatus,
    section_focus: LayoutSections,
    task_list_scroll: TableState,
    selected_task_index: usize,
    is_task_selection_pinned: bool,
    showing_help_popup: bool,
    done: bool,
    preferences: PreferenceLoader,
    scrollback_len: u64,
    scroll_momentum: ScrollMomentum,
}

impl<W> App<W> {
    pub fn new(
        rows: u16,
        cols: u16,
        tasks: Vec<String>,
        preferences: PreferenceLoader,
        scrollback_len: u64,
    ) -> Self {
        debug!("tasks: {tasks:?}");
        let size = SizeInfo::new(rows, cols, tasks.iter().map(|s| s.as_str()));

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

        let pane_rows = size.pane_rows();
        let pane_cols = size.pane_cols();

        // Attempt to load previous selection. If there isn't one, go to index 0.
        let selected_task_index = preferences
            .active_task()
            .and_then(|active_task| tasks_by_status.active_index(active_task))
            .unwrap_or(0);

        Self {
            size,
            done: false,
            section_focus: LayoutSections::TaskList,
            tasks: tasks_by_status
                .task_names_in_displayed_order()
                .map(|task_name| {
                    (
                        task_name.to_owned(),
                        TerminalOutput::new(pane_rows, pane_cols, None, scrollback_len),
                    )
                })
                .collect(),
            selected_task_index,
            tasks_by_status,
            task_list_scroll: TableState::default().with_selected(selected_task_index),
            showing_help_popup: false,
            is_task_selection_pinned: preferences.active_task().is_some(),
            preferences,
            scrollback_len,
            scroll_momentum: ScrollMomentum::new(),
        }
    }

    #[cfg(test)]
    fn is_focusing_pane(&self) -> bool {
        match self.section_focus {
            LayoutSections::Pane => true,
            LayoutSections::TaskList
            | LayoutSections::Search { .. }
            | LayoutSections::SearchLocked { .. } => false,
        }
    }

    pub fn active_task(&self) -> Result<&str, Error> {
        self.tasks_by_status.task_name(self.selected_task_index)
    }

    fn input_options(&self) -> Result<InputOptions<'_>, Error> {
        let has_selection = self.get_full_task()?.has_selection();
        Ok(InputOptions {
            focus: &self.section_focus,
            has_selection,
            is_help_popup_open: self.showing_help_popup,
        })
    }

    fn update_sidebar_toggle(&mut self) {
        let value = !self.preferences.is_task_list_visible();
        self.preferences.set_is_task_list_visible(Some(value));
    }

    fn update_task_selection_pinned_state(&mut self) -> Result<(), Error> {
        // Preferences assume a pinned state when there is an active task.
        // This `None` creates "un-pinned-ness" on the next TUI startup.
        self.preferences.set_active_task(None);
        Ok(())
    }

    pub fn get_full_task(&self) -> Result<&TerminalOutput<W>, Error> {
        let active_task = self.active_task()?;
        self.tasks
            .get(active_task)
            .ok_or_else(|| Error::TaskNotFound {
                name: active_task.to_owned(),
            })
    }

    pub fn get_full_task_mut(&mut self) -> Result<&mut TerminalOutput<W>, Error> {
        // Clippy is wrong here, we need this to avoid a borrow checker error
        #[allow(clippy::unnecessary_to_owned)]
        let active_task = self.active_task()?.to_owned();
        self.tasks
            .get_mut(&active_task)
            .ok_or(Error::TaskNotFound { name: active_task })
    }

    fn persist_active_task(&mut self) -> Result<(), Error> {
        let active_task = self.active_task()?;
        self.preferences.set_active_task(
            self.is_task_selection_pinned
                .then(|| active_task.to_owned()),
        );
        Ok(())
    }

    /// Navigate to the next matching task in locked search mode, with
    /// wrap-around
    fn next_matching_task(&mut self, results: SearchResults) {
        let next_task = results
            .first_match(
                self.tasks_by_status
                    .task_names_in_displayed_order()
                    .skip(self.selected_task_index + 1),
            )
            .or_else(|| {
                // Wrap around to first match
                results.first_match(self.tasks_by_status.task_names_in_displayed_order())
            })
            .map(str::to_owned);

        if let Some(task) = next_task {
            // Safe to ignore error as we're in a navigation flow
            let _ = self.select_task(&task);
        }
    }

    /// Navigate to the previous matching task in locked search mode, with
    /// wrap-around
    fn previous_matching_task(&mut self, results: SearchResults) {
        let num_rows = self.tasks_by_status.count_all();
        let prev_task = results
            .first_match(
                self.tasks_by_status
                    .task_names_in_displayed_order()
                    .rev()
                    .skip(num_rows - self.selected_task_index),
            )
            .or_else(|| {
                // Wrap around to last match
                results.first_match(self.tasks_by_status.task_names_in_displayed_order().rev())
            })
            .map(str::to_owned);

        if let Some(task) = prev_task {
            // Safe to ignore error as we're in a navigation flow
            let _ = self.select_task(&task);
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn next(&mut self) {
        let num_rows = self.tasks_by_status.count_all();
        if num_rows == 0 {
            return;
        }

        if let LayoutSections::SearchLocked { results } = &self.section_focus {
            self.next_matching_task(results.clone());
        } else {
            self.selected_task_index = (self.selected_task_index + 1) % num_rows;
            self.task_list_scroll.select(Some(self.selected_task_index));
        }

        self.is_task_selection_pinned = true;
        self.persist_active_task().ok();
    }

    #[tracing::instrument(skip(self))]
    pub fn previous(&mut self) {
        let num_rows = self.tasks_by_status.count_all();
        if num_rows == 0 {
            return;
        }

        if let LayoutSections::SearchLocked { results } = &self.section_focus {
            self.previous_matching_task(results.clone());
        } else {
            self.selected_task_index = self
                .selected_task_index
                .checked_sub(1)
                .unwrap_or(num_rows - 1);
            self.task_list_scroll.select(Some(self.selected_task_index));
        }

        self.is_task_selection_pinned = true;
        self.persist_active_task().ok();
    }

    #[tracing::instrument(skip_all)]
    pub fn scroll_terminal_output(
        &mut self,
        direction: Direction,
        use_momentum: bool,
    ) -> Result<(), Error> {
        let lines = if use_momentum {
            self.scroll_momentum.on_scroll_event(direction)
        } else {
            self.scroll_momentum.reset();
            1
        };

        if lines > 0 {
            self.get_full_task_mut()?.scroll_by(direction, lines)?;
        }
        Ok(())
    }

    pub fn scroll_terminal_output_by_page(&mut self, direction: Direction) -> Result<(), Error> {
        let pane_rows = self.size.pane_rows();
        let task = self.get_full_task_mut()?;
        // Scroll by the height of the terminal pane
        task.scroll_by(direction, usize::from(pane_rows))?;

        Ok(())
    }

    pub fn enter_search(&mut self) -> Result<(), Error> {
        // Ensure task list is visible when searching
        if !self.preferences.is_task_list_visible() {
            self.preferences.set_is_task_list_visible(Some(true));
        }

        self.section_focus = LayoutSections::Search {
            previous_selection: self.active_task()?.to_string(),
            results: SearchResults::new(&self.tasks_by_status),
        };
        // We set scroll as we want to keep the current selection
        self.is_task_selection_pinned = true;
        Ok(())
    }

    pub fn exit_search(&mut self, restore_scroll: bool) {
        let mut prev_focus = LayoutSections::TaskList;
        mem::swap(&mut self.section_focus, &mut prev_focus);
        match prev_focus {
            LayoutSections::Search {
                previous_selection, ..
            } if restore_scroll => {
                if self.select_task(&previous_selection).is_err() {
                    // If the task that was selected is no longer in the task list we reset
                    // scrolling.
                    self.reset_scroll();
                }
            }
            _ => {}
        }
    }

    pub fn lock_search(&mut self) {
        if let LayoutSections::Search { results, .. } = &self.section_focus {
            self.section_focus = LayoutSections::SearchLocked {
                results: results.clone(),
            };
        }
    }

    pub fn search_scroll(&mut self, direction: Direction) -> Result<(), Error> {
        let LayoutSections::Search { results, .. } = &self.section_focus else {
            debug!("scrolling search while not searching");
            return Ok(());
        };
        let results = results.clone();

        match direction {
            Direction::Down => self.next_matching_task(results),
            Direction::Up => self.previous_matching_task(results),
        }

        Ok(())
    }

    pub fn search_enter_char(&mut self, c: char) -> Result<(), Error> {
        let LayoutSections::Search { results, .. } = &mut self.section_focus else {
            debug!("modifying search query while not searching");
            return Ok(());
        };
        results.modify_query(|s| s.push(c));
        self.update_search_results();
        Ok(())
    }

    pub fn search_remove_char(&mut self) -> Result<(), Error> {
        let LayoutSections::Search { results, .. } = &mut self.section_focus else {
            debug!("modified search query while not searching");
            return Ok(());
        };
        let mut query_was_empty = false;
        results.modify_query(|s| {
            query_was_empty = s.pop().is_none();
        });
        if query_was_empty {
            self.exit_search(true);
        } else {
            self.update_search_results();
        }
        Ok(())
    }

    fn update_search_results(&mut self) {
        let LayoutSections::Search { results, .. } = &self.section_focus else {
            return;
        };

        // if currently selected task is in results stay on it
        // if not we go forward looking for a task in results
        if let Some(result) = results
            .first_match(
                self.tasks_by_status
                    .task_names_in_displayed_order()
                    .skip(self.selected_task_index),
            )
            .or_else(|| results.first_match(self.tasks_by_status.task_names_in_displayed_order()))
        {
            let new_selection = result.to_owned();
            self.is_task_selection_pinned = true;
            self.select_task(&new_selection).expect("todo");
        }
    }

    /// Mark the given task as started.
    /// If planned, pulls it from planned tasks and starts it.
    /// If finished, removes from finished and starts again as new task.
    #[tracing::instrument(skip(self, output_logs))]
    pub fn start_task(&mut self, task: &str, output_logs: OutputLogs) -> Result<(), Error> {
        debug!("starting {task}");
        // Name of currently highlighted task.
        // We will use this after the order switches.
        let highlighted_task = self
            .tasks_by_status
            .task_name(self.selected_task_index)?
            .to_string();

        let mut found_task = false;

        if let Some(planned_idx) = self
            .tasks_by_status
            .planned
            .iter()
            .position(|planned| planned.name() == task)
        {
            let planned = self.tasks_by_status.planned.remove(planned_idx);
            let running = planned.start();
            self.tasks_by_status.running.push(running);

            found_task = true;
        }

        if !found_task {
            return Err(Error::TaskNotFound { name: task.into() });
        }
        self.tasks
            .get_mut(task)
            .ok_or_else(|| Error::TaskNotFound { name: task.into() })?
            .output_logs = Some(output_logs);

        // If user hasn't interacted, keep highlighting top-most task in list.
        self.select_task(&highlighted_task)?;

        Ok(())
    }

    /// Mark the given running task as finished
    /// Errors if given task wasn't a running task
    #[tracing::instrument(skip(self, result))]
    pub fn finish_task(&mut self, task: &str, result: TaskResult) -> Result<(), Error> {
        debug!("finishing task {task}");
        // Name of currently highlighted task.
        // We will use this after the order switches.
        let highlighted_task = self
            .tasks_by_status
            .task_name(self.selected_task_index)?
            .to_string();

        let running_idx = self
            .tasks_by_status
            .running
            .iter()
            .position(|running| running.name() == task)
            .ok_or_else(|| Error::TaskNotFound { name: task.into() })?;

        let running = self.tasks_by_status.running.remove(running_idx);
        self.tasks_by_status
            .insert_finished_task(running.finish(result));

        self.tasks
            .get_mut(task)
            .ok_or_else(|| Error::TaskNotFound { name: task.into() })?
            .task_result = Some(result);

        // Find the highlighted task from before the list movement in the new list.
        self.select_task(&highlighted_task)?;

        Ok(())
    }

    pub fn has_stdin(&self) -> Result<bool, Error> {
        if let Some(term) = self.tasks.get(self.active_task()?) {
            Ok(term.stdin.is_some())
        } else {
            Ok(false)
        }
    }

    pub fn interact(&mut self) -> Result<(), Error> {
        if matches!(self.section_focus, LayoutSections::Pane) {
            self.section_focus = LayoutSections::TaskList
        } else if self.has_stdin()? {
            self.section_focus = LayoutSections::Pane;
        }
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub fn update_tasks(&mut self, tasks: Vec<String>) -> Result<(), Error> {
        if tasks.is_empty() {
            debug!("got request to update task list to empty list, ignoring request");
            return Ok(());
        }
        debug!("updating task list: {tasks:?}");
        let highlighted_task = self.active_task()?.to_owned();
        // Make sure all tasks have a terminal output
        for task in &tasks {
            self.tasks.entry(task.clone()).or_insert_with(|| {
                TerminalOutput::new(
                    self.size.pane_rows(),
                    self.size.pane_cols(),
                    None,
                    self.scrollback_len,
                )
            });
        }
        // Trim the terminal output to only tasks that exist in new list
        self.tasks.retain(|name, _| tasks.contains(name));
        // Update task list
        let mut task_list = tasks.into_iter().map(Task::new).collect::<Vec<_>>();
        task_list.sort_unstable();
        task_list.dedup();
        self.tasks_by_status = TasksByStatus {
            planned: task_list,
            running: Default::default(),
            finished: Default::default(),
        };

        // Task that was selected may have been removed, go back to top if this happens
        if self.select_task(&highlighted_task).is_err() {
            trace!("{highlighted_task} was removed from list");
            self.reset_scroll();
        }

        match &mut self.section_focus {
            LayoutSections::Search { results, .. }
            | LayoutSections::SearchLocked { results, .. } => {
                results.update_tasks(&self.tasks_by_status);
            }
            _ => {}
        }
        self.update_search_results();

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub fn restart_tasks(&mut self, tasks: Vec<String>) -> Result<(), Error> {
        debug!("tasks to reset: {tasks:?}");
        let highlighted_task = self.active_task()?.to_owned();
        // Make sure all tasks have a terminal output
        for task in &tasks {
            self.tasks.entry(task.clone()).or_insert_with(|| {
                TerminalOutput::new(
                    self.size.pane_rows(),
                    self.size.pane_cols(),
                    None,
                    self.scrollback_len,
                )
            });
        }

        self.tasks_by_status
            .restart_tasks(tasks.iter().map(|s| s.as_str()));

        match &mut self.section_focus {
            LayoutSections::Search { results, .. }
            | LayoutSections::SearchLocked { results, .. } => {
                results.update_tasks(&self.tasks_by_status);
            }
            _ => {}
        }

        if self.select_task(&highlighted_task).is_err() {
            debug!("was unable to find {highlighted_task} after restart");
            self.reset_scroll();
        }

        Ok(())
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

    #[tracing::instrument(skip(self))]
    pub fn set_status(
        &mut self,
        task: String,
        status: String,
        result: CacheResult,
    ) -> Result<(), Error> {
        let task = self
            .tasks
            .get_mut(&task)
            .ok_or_else(|| Error::TaskNotFound {
                name: task.to_owned(),
            })?;
        task.status = Some(status);
        task.cache_result = Some(result);
        Ok(())
    }

    pub fn handle_mouse(&mut self, mut event: crossterm::event::MouseEvent) -> Result<(), Error> {
        let table_width = self.size.task_list_width();
        debug!("original mouse event: {event:?}, table_width: {table_width}");
        // Only handle mouse event if it happens inside of pane
        // We give a 1 cell buffer to make it easier to select the first column of a row
        if event.row > 0 && event.column >= table_width {
            // Subtract 1 from the y axis due to the title of the pane
            event.row -= 1;
            // Subtract the width of the table
            event.column -= table_width;
            debug!("translated mouse event: {event:?}");

            let task = self.get_full_task_mut()?;
            task.handle_mouse(event)?;
        }

        Ok(())
    }

    pub fn copy_selection(&self) -> Result<(), Error> {
        let task = self.get_full_task()?;
        let Some(text) = task.copy_selection() else {
            return Ok(());
        };
        super::copy_to_clipboard(&text);
        Ok(())
    }

    fn select_task(&mut self, task_name: &str) -> Result<(), Error> {
        if !self.is_task_selection_pinned {
            return Ok(());
        }

        let Some(new_index_to_highlight) = self
            .tasks_by_status
            .task_names_in_displayed_order()
            .position(|task| task == task_name)
        else {
            return Err(Error::TaskNotFound {
                name: task_name.to_owned(),
            });
        };
        self.selected_task_index = new_index_to_highlight;
        self.task_list_scroll.select(Some(new_index_to_highlight));

        Ok(())
    }

    /// Resets scroll state
    pub fn reset_scroll(&mut self) {
        self.is_task_selection_pinned = false;
        self.task_list_scroll.select(Some(0));
        self.selected_task_index = 0;
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.size.resize(rows, cols);
        let pane_rows = self.size.pane_rows();
        let pane_cols = self.size.pane_cols();
        self.tasks.values_mut().for_each(|term| {
            term.resize(pane_rows, pane_cols);
        })
    }

    pub fn jump_to_logs_top(&mut self) -> Result<(), Error> {
        let task = self.get_full_task_mut()?;
        task.parser.screen_mut().set_scrollback(usize::MAX);
        Ok(())
    }

    pub fn jump_to_logs_bottom(&mut self) -> Result<(), Error> {
        let task = self.get_full_task_mut()?;
        task.parser.screen_mut().set_scrollback(0);
        Ok(())
    }

    pub fn clear_task_logs(&mut self) -> Result<(), Error> {
        let task = self.get_full_task_mut()?;
        task.clear_logs();
        Ok(())
    }
}

impl<W: Write> App<W> {
    /// Insert a stdin to be associated with a task
    pub fn insert_stdin(&mut self, task: &str, stdin: Option<W>) -> Result<(), Error> {
        let task = self
            .tasks
            .get_mut(task)
            .ok_or_else(|| Error::TaskNotFound {
                name: task.to_owned(),
            })?;
        task.stdin = stdin;
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub fn forward_input(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if matches!(self.section_focus, LayoutSections::Pane) {
            let task_output = self.get_full_task_mut()?;
            if let Some(stdin) = &mut task_output.stdin {
                stdin.write_all(bytes).map_err(|e| Error::Stdin {
                    name: self
                        .active_task()
                        .unwrap_or("<unable to retrieve task name>")
                        .to_owned(),
                    e,
                })?;
            }
            Ok(())
        } else {
            Ok(())
        }
    }

    #[tracing::instrument(skip(self, output))]
    pub fn process_output(&mut self, task: &str, output: &[u8]) -> Result<(), Error> {
        let task_output = self
            .tasks
            .get_mut(task)
            .ok_or_else(|| Error::TaskNotFound {
                name: task.to_owned(),
            })?;
        task_output.process(output);
        Ok(())
    }
}

/// Handle the rendering of the `App` widget based on events received by
/// `receiver`
pub async fn run_app(
    tasks: Vec<String>,
    receiver: AppReceiver,
    color_config: ColorConfig,
    repo_root: &AbsoluteSystemPathBuf,
    scrollback_len: u64,
) -> Result<(), Error> {
    let mut terminal = startup(color_config)?;
    let size = terminal.size()?;
    let preferences = PreferenceLoader::new(repo_root);

    let mut app: App<Box<dyn io::Write + Send>> =
        App::new(size.height, size.width, tasks, preferences, scrollback_len);
    let (crossterm_tx, crossterm_rx) = mpsc::channel(1024);
    input::start_crossterm_stream(crossterm_tx);

    let (result, callback) =
        match run_app_inner(&mut terminal, &mut app, receiver, crossterm_rx).await {
            Ok(callback) => (Ok(()), callback),
            Err(err) => {
                debug!("tui shutting down: {err}");
                (Err(err), None)
            }
        };

    cleanup(terminal, app, callback)?;

    result
}

// Break out inner loop so we can use `?` without worrying about cleaning up the
// terminal.
async fn run_app_inner<B: Backend + std::io::Write>(
    terminal: &mut Terminal<B>,
    app: &mut App<Box<dyn io::Write + Send>>,
    mut receiver: AppReceiver,
    mut crossterm_rx: mpsc::Receiver<crossterm::event::Event>,
) -> Result<Option<oneshot::Sender<()>>, Error> {
    // Render initial state to paint the screen
    terminal.draw(|f| view(app, f))?;
    let mut last_render = Instant::now();
    let mut resize_debouncer = Debouncer::new(RESIZE_DEBOUNCE_DELAY);
    let mut callback = None;
    let mut needs_rerender = true;
    while let Some(event) = poll(app.input_options()?, &mut receiver, &mut crossterm_rx).await {
        // If we only receive ticks, then there's been no state change so no update
        // needed
        if !matches!(event, Event::Tick) {
            needs_rerender = true;
        }

        let mut event = Some(event);
        let mut resize_event = None;
        if matches!(event, Some(Event::Resize { .. })) {
            resize_event = resize_debouncer.update(
                event
                    .take()
                    .expect("we just matched against a present value"),
            );
        }
        if let Some(resize) = resize_event.take().or_else(|| resize_debouncer.query()) {
            // If we got a resize event, make sure to update ratatui backend.
            terminal.autoresize()?;
            update(app, resize)?;
        }
        if let Some(event) = event {
            callback = update(app, event)?;
            if app.done {
                break;
            }
            if FRAMERATE <= last_render.elapsed() && needs_rerender {
                terminal.draw(|f| view(app, f))?;
                last_render = Instant::now();
                needs_rerender = false;
            }
        }
    }

    Ok(callback)
}

/// Blocking poll for events, will only return None if app handle has been
/// dropped
async fn poll(
    input_options: InputOptions<'_>,
    receiver: &mut AppReceiver,
    crossterm_rx: &mut mpsc::Receiver<crossterm::event::Event>,
) -> Option<Event> {
    let input_closed = crossterm_rx.is_closed();

    if input_closed {
        receiver.recv().await
    } else {
        // tokio::select is messing with variable read detection
        #[allow(unused_assignments)]
        let mut event = None;
        loop {
            tokio::select! {
                e = crossterm_rx.recv() => {
                    event = e.and_then(|e| input_options.handle_crossterm_event(e));
                }
                e = receiver.recv() => {
                    event = e;
                }
            }
            if event.is_some() {
                break;
            }
        }
        event
    }
}

const MIN_HEIGHT: u16 = 10;
const MIN_WIDTH: u16 = 20;

pub fn terminal_big_enough() -> Result<bool, Error> {
    let (width, height) = crossterm::terminal::size()?;
    Ok(width >= MIN_WIDTH && height >= MIN_HEIGHT)
}

/// Configures terminal for rendering App
#[tracing::instrument]
fn startup(color_config: ColorConfig) -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    if color_config.should_strip_ansi {
        crossterm::style::force_color_output(false);
    }
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
#[tracing::instrument(skip_all)]
fn cleanup<B: Backend + io::Write>(
    mut terminal: Terminal<B>,
    mut app: App<Box<dyn io::Write + Send>>,
    callback: Option<oneshot::Sender<()>>,
) -> io::Result<()> {
    terminal.clear()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen,
    )?;
    let tasks_started = app.tasks_by_status.tasks_started();
    app.persist_tasks(tasks_started)?;
    app.preferences.flush_to_disk().ok();
    crossterm::terminal::disable_raw_mode()?;
    terminal.show_cursor()?;
    // We can close the channel now that terminal is back restored to a normal state
    drop(callback);
    Ok(())
}

fn update(
    app: &mut App<Box<dyn io::Write + Send>>,
    event: Event,
) -> Result<Option<oneshot::Sender<()>>, Error> {
    match event {
        Event::StartTask { task, output_logs } => {
            app.start_task(&task, output_logs)?;
        }
        Event::TaskOutput { task, output } => {
            app.process_output(&task, &output)?;
        }
        Event::Status {
            task,
            status,
            result,
        } => {
            app.set_status(task, status, result)?;
        }
        Event::InternalStop => {
            debug!("shutting down due to internal failure");
            app.done = true;
        }
        Event::Stop(callback) => {
            debug!("shutting down due to message");
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
            app.is_task_selection_pinned = true;
            app.scroll_momentum.reset();
            app.scroll_terminal_output(Direction::Up, false)?
        }
        Event::ScrollDown => {
            app.is_task_selection_pinned = true;
            app.scroll_momentum.reset();
            app.scroll_terminal_output(Direction::Down, false)?;
        }
        Event::ScrollWithMomentum(direction) => {
            app.is_task_selection_pinned = true;
            app.scroll_terminal_output(direction, true)?;
        }
        Event::PageUp => {
            app.is_task_selection_pinned = true;
            app.scroll_momentum.reset();
            app.scroll_terminal_output_by_page(Direction::Up)?;
        }
        Event::PageDown => {
            app.is_task_selection_pinned = true;
            app.scroll_momentum.reset();
            app.scroll_terminal_output_by_page(Direction::Down)?;
        }
        Event::JumpToLogsTop => {
            app.is_task_selection_pinned = true;
            app.scroll_momentum.reset();
            app.jump_to_logs_top()?;
        }
        Event::JumpToLogsBottom => {
            app.is_task_selection_pinned = true;
            app.scroll_momentum.reset();
            app.jump_to_logs_bottom()?;
        }
        Event::ClearLogs => {
            app.clear_task_logs()?;
        }
        Event::EnterInteractive => {
            app.is_task_selection_pinned = true;
            app.interact()?;
        }
        Event::ExitInteractive => {
            app.is_task_selection_pinned = true;
            app.interact()?;
        }
        Event::TogglePinnedTask => {
            app.update_task_selection_pinned_state()?;
        }
        Event::ToggleSidebar => {
            app.update_sidebar_toggle();
        }
        Event::ToggleHelpPopup => {
            app.showing_help_popup = !app.showing_help_popup;
        }
        Event::Input { bytes } => {
            app.forward_input(&bytes)?;
        }
        Event::SetStdin { task, stdin } => {
            app.insert_stdin(&task, Some(stdin))?;
        }
        Event::UpdateTasks { tasks } => {
            app.update_tasks(tasks)?;
        }
        Event::Mouse(m) => {
            app.handle_mouse(m)?;
        }
        Event::CopySelection => {
            app.copy_selection()?;
        }
        Event::RestartTasks { tasks } => {
            app.restart_tasks(tasks)?;
        }
        Event::Resize { rows, cols } => {
            app.resize(rows, cols);
        }
        Event::SearchEnter => {
            app.enter_search()?;
        }
        Event::SearchExit { restore_scroll } => {
            app.exit_search(restore_scroll);
        }
        Event::SearchLock => {
            app.lock_search();
        }
        Event::SearchScroll { direction } => {
            app.search_scroll(direction)?;
        }
        Event::SearchEnterChar(c) => {
            app.search_enter_char(c)?;
        }
        Event::SearchBackspace => {
            app.search_remove_char()?;
        }
        Event::PaneSizeQuery(callback) => {
            // If caller has already hung up do nothing
            callback
                .send(PaneSize {
                    rows: app.size.pane_rows(),
                    cols: app.size.pane_cols(),
                })
                .ok();
        }
    }
    Ok(None)
}

fn view<W>(app: &mut App<W>, f: &mut Frame) {
    let cols = app.size.pane_cols();
    let horizontal = if app.preferences.is_task_list_visible() {
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(cols)])
    } else {
        Layout::horizontal([Constraint::Max(0), Constraint::Length(cols)])
    };
    let [table, pane] = horizontal.areas(f.size());

    let active_task = app.active_task().unwrap().to_string();

    let output_logs = app.tasks.get(&active_task).unwrap();
    let pane_to_render: TerminalPane<W> = TerminalPane::new(
        output_logs,
        &active_task,
        &app.section_focus,
        app.preferences.is_task_list_visible(),
    );

    let table_to_render = TaskTable::new(&app.tasks_by_status, &app.section_focus);

    f.render_stateful_widget(&table_to_render, table, &mut app.task_list_scroll);
    f.render_widget(&pane_to_render, pane);

    if app.showing_help_popup {
        let area = popup_area(*f.buffer_mut().area());
        let area = area.intersection(*f.buffer_mut().area());
        f.render_widget(Clear, area); // Clears background underneath popup
        f.render_widget(popup(area), area);
    }
}

#[cfg(test)]
mod test {
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;
    use crate::tui::event::CacheResult;

    #[test]
    fn test_scroll() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<bool> = App::new(
            100,
            100,
            vec!["foo".to_string(), "bar".to_string(), "baz".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        assert_eq!(
            app.task_list_scroll.selected(),
            Some(0),
            "starts with first selection"
        );
        app.next();
        assert_eq!(
            app.task_list_scroll.selected(),
            Some(1),
            "scroll starts from 0 and goes to 1"
        );
        app.previous();
        assert_eq!(
            app.task_list_scroll.selected(),
            Some(0),
            "scroll stays in bounds"
        );
        app.next();
        app.next();
        assert_eq!(
            app.task_list_scroll.selected(),
            Some(2),
            "scroll moves forwards"
        );
        app.next();
        assert_eq!(app.task_list_scroll.selected(), Some(0), "scroll wraps");
        app.previous();
        assert_eq!(
            app.task_list_scroll.selected(),
            Some(2),
            "scroll stays in bounds"
        );
        Ok(())
    }

    #[test]
    fn test_selection_follows() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<bool> = App::new(
            100,
            100,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.next();
        assert_eq!(app.task_list_scroll.selected(), Some(1), "selected b");
        assert_eq!(app.active_task()?, "b", "selected b");
        app.start_task("b", OutputLogs::Full).unwrap();
        assert_eq!(app.task_list_scroll.selected(), Some(0), "b stays selected");
        assert_eq!(app.active_task()?, "b", "selected b");
        app.start_task("a", OutputLogs::Full).unwrap();
        assert_eq!(app.task_list_scroll.selected(), Some(0), "b stays selected");
        assert_eq!(app.active_task()?, "b", "selected b");
        app.finish_task("a", TaskResult::Success).unwrap();
        assert_eq!(app.task_list_scroll.selected(), Some(0), "b stays selected");
        assert_eq!(app.active_task()?, "b", "selected b");
        Ok(())
    }

    #[test]
    fn test_restart_task() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.next();
        app.next();
        // Start all tasks
        app.start_task("b", OutputLogs::Full)?;
        app.start_task("a", OutputLogs::Full)?;
        app.start_task("c", OutputLogs::Full)?;
        assert_eq!(
            app.tasks_by_status.task_name(0)?,
            "b",
            "b is on top (running)"
        );
        app.finish_task("a", TaskResult::Success)?;
        assert_eq!(
            (
                app.tasks_by_status.task_name(2)?,
                app.tasks_by_status.task_name(0)?
            ),
            ("a", "b"),
            "a is on bottom (done), b is second (running)"
        );

        app.finish_task("b", TaskResult::Success)?;
        assert_eq!(
            (
                app.tasks_by_status.task_name(1)?,
                app.tasks_by_status.task_name(2)?
            ),
            ("a", "b"),
            "a is second (done), b is last (done)"
        );

        // Restart b
        app.restart_tasks(vec!["b".to_string()])?;
        app.start_task("b", OutputLogs::Full)?;
        assert_eq!(
            (
                app.tasks_by_status.task_name(1)?,
                app.tasks_by_status.task_name(0)?
            ),
            ("b", "c"),
            "b is second (running), c is first (running)"
        );

        // Restart a
        app.restart_tasks(vec!["a".to_string()])?;
        app.start_task("a", OutputLogs::Full)?;
        assert_eq!(
            (
                app.tasks_by_status.task_name(0)?,
                app.tasks_by_status.task_name(1)?,
                app.tasks_by_status.task_name(2)?
            ),
            ("c", "b", "a"),
            "c is on top (running), b is second (running), a is third
        (running)"
        );
        Ok(())
    }

    #[test]
    fn test_selection_stable() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<bool> = App::new(
            100,
            100,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.next();
        app.next();
        assert_eq!(app.task_list_scroll.selected(), Some(2), "selected c");
        assert_eq!(app.tasks_by_status.task_name(2)?, "c", "selected c");
        // start c which moves it to "running" which is before "planned"
        app.start_task("c", OutputLogs::Full)?;
        assert_eq!(
            app.task_list_scroll.selected(),
            Some(0),
            "selection stays on c"
        );
        assert_eq!(app.tasks_by_status.task_name(0)?, "c", "selected c");
        app.start_task("a", OutputLogs::Full)?;
        assert_eq!(
            app.task_list_scroll.selected(),
            Some(0),
            "selection stays on c"
        );
        assert_eq!(app.tasks_by_status.task_name(0)?, "c", "selected c");
        // c
        // a
        // b <-
        app.next();
        app.next();
        assert_eq!(app.task_list_scroll.selected(), Some(2), "selected b");
        assert_eq!(app.tasks_by_status.task_name(2)?, "b", "selected b");
        app.finish_task("a", TaskResult::Success)?;
        assert_eq!(app.task_list_scroll.selected(), Some(1), "b stays selected");
        assert_eq!(app.tasks_by_status.task_name(1)?, "b", "selected b");
        // c <-
        // b
        // a
        app.previous();
        app.finish_task("c", TaskResult::Success)?;
        assert_eq!(app.task_list_scroll.selected(), Some(2), "c stays selected");
        assert_eq!(app.tasks_by_status.task_name(2)?, "c", "selected c");
        Ok(())
    }

    #[test]
    fn test_forward_stdin() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<Vec<u8>> = App::new(
            100,
            100,
            vec!["a".to_string(), "b".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.next();
        assert_eq!(app.task_list_scroll.selected(), Some(1), "selected b");
        assert_eq!(app.tasks_by_status.task_name(1)?, "b", "selected b");
        // start c which moves it to "running" which is before "planned"
        app.start_task("a", OutputLogs::Full)?;
        app.start_task("b", OutputLogs::Full)?;
        app.insert_stdin("a", Some(Vec::new())).unwrap();
        app.insert_stdin("b", Some(Vec::new())).unwrap();

        // Interact and type "hello"
        app.interact()?;
        app.forward_input(b"hello!").unwrap();

        // Exit interaction and move up
        app.interact()?;
        app.previous();
        app.interact()?;
        app.forward_input(b"world").unwrap();

        assert_eq!(
            app.tasks.get("b").unwrap().stdin.as_deref().unwrap(),
            b"hello!"
        );
        assert_eq!(
            app.tasks.get("a").unwrap().stdin.as_deref().unwrap(),
            b"world"
        );
        Ok(())
    }

    #[test]
    fn test_interact() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<Vec<u8>> = App::new(
            100,
            100,
            vec!["a".to_string(), "b".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        assert!(!app.is_focusing_pane(), "app starts focused on table");
        app.insert_stdin("a", Some(Vec::new()))?;

        app.interact()?;
        assert!(app.is_focusing_pane(), "can focus pane when task has stdin");

        app.interact()?;
        assert!(
            !app.is_focusing_pane(),
            "interact changes focus to table if focused on pane"
        );

        app.next();
        assert!(!app.is_focusing_pane(), "pane isn't focused after move");
        app.interact()?;
        assert!(!app.is_focusing_pane(), "cannot focus task without stdin");
        Ok(())
    }

    #[test]
    fn test_task_status() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<Vec<u8>> = App::new(
            100,
            100,
            vec!["a".to_string(), "b".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.next();
        assert_eq!(app.task_list_scroll.selected(), Some(1), "selected b");
        assert_eq!(app.tasks_by_status.task_name(1)?, "b", "selected b");
        // set status for a
        app.set_status("a".to_string(), "building".to_string(), CacheResult::Hit)?;

        assert_eq!(
            app.tasks.get("a").unwrap().status.as_deref(),
            Some("building")
        );
        assert!(app.tasks.get("b").unwrap().status.is_none());
        Ok(())
    }

    #[test]
    fn test_restarting_task_no_scroll() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        assert_eq!(app.task_list_scroll.selected(), Some(0), "selected a");
        assert_eq!(app.tasks_by_status.task_name(0)?, "a", "selected a");
        app.start_task("a", OutputLogs::None)?;
        app.start_task("b", OutputLogs::None)?;
        app.start_task("c", OutputLogs::None)?;
        app.finish_task("b", TaskResult::Success)?;
        app.finish_task("c", TaskResult::Success)?;
        app.finish_task("a", TaskResult::Success)?;

        assert_eq!(app.task_list_scroll.selected(), Some(0), "selected b");
        assert_eq!(app.tasks_by_status.task_name(0)?, "b", "selected b");

        app.restart_tasks(vec!["c".to_string()])?;

        assert_eq!(
            app.tasks_by_status
                .task_name(app.task_list_scroll.selected().unwrap())?,
            "c",
            "selected c"
        );
        Ok(())
    }

    #[test]
    fn test_restarting_task() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.next();
        assert_eq!(app.task_list_scroll.selected(), Some(1), "selected b");
        assert_eq!(app.tasks_by_status.task_name(1)?, "b", "selected b");
        app.start_task("a", OutputLogs::None)?;
        app.start_task("b", OutputLogs::None)?;
        app.start_task("c", OutputLogs::None)?;
        app.finish_task("b", TaskResult::Success)?;
        app.finish_task("c", TaskResult::Success)?;
        app.finish_task("a", TaskResult::Success)?;

        assert_eq!(app.task_list_scroll.selected(), Some(0), "selected b");
        assert_eq!(app.tasks_by_status.task_name(0)?, "b", "selected b");

        app.restart_tasks(vec!["c".to_string()])?;

        assert_eq!(
            app.tasks_by_status
                .task_name(app.task_list_scroll.selected().unwrap())?,
            "b",
            "selected b"
        );
        Ok(())
    }

    #[test]
    fn test_resize() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<Vec<u8>> = App::new(
            20,
            24,
            vec!["a".to_string(), "b".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        let pane_rows = app.size.pane_rows();
        let pane_cols = app.size.pane_cols();
        for (name, task) in app.tasks.iter() {
            let (rows, cols) = task.size();
            assert_eq!(
                (rows, cols),
                (pane_rows, pane_cols),
                "size mismatch for {name}"
            );
        }

        app.resize(20, 18);
        let new_pane_rows = app.size.pane_rows();
        let new_pane_cols = app.size.pane_cols();
        assert_eq!(pane_rows, new_pane_rows);
        assert_ne!(pane_cols, new_pane_cols);

        for (name, task) in app.tasks.iter() {
            let (rows, cols) = task.size();
            assert_eq!(
                (rows, cols),
                (new_pane_rows, new_pane_cols),
                "size mismatch for {name}"
            );
        }
        Ok(())
    }

    #[test]
    fn test_update_empty_task_list() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.next();
        app.update_tasks(Vec::new())?;

        assert_eq!(app.active_task()?, "b", "selected b");
        Ok(())
    }

    #[test]
    fn test_restart_missing_task() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.next();
        app.restart_tasks(vec!["d".to_string()])?;

        assert_eq!(app.active_task()?, "b", "selected b");

        app.start_task("d", OutputLogs::Full)?;
        Ok(())
    }

    #[test]
    fn test_search_backspace_exits_search() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.enter_search()?;
        assert!(matches!(app.section_focus, LayoutSections::Search { .. }));
        app.search_remove_char()?;
        assert!(matches!(app.section_focus, LayoutSections::TaskList));
        app.enter_search()?;
        app.search_enter_char('a')?;
        assert!(matches!(app.section_focus, LayoutSections::Search { .. }));
        app.search_remove_char()?;
        assert!(matches!(app.section_focus, LayoutSections::Search { .. }));
        app.search_remove_char()?;
        assert!(matches!(app.section_focus, LayoutSections::TaskList));
        Ok(())
    }

    #[test]
    fn test_search_moves_with_typing() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "ab".to_string(), "abc".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.enter_search()?;
        app.search_enter_char('a')?;
        assert_eq!(app.active_task()?, "a");
        app.search_enter_char('b')?;
        assert_eq!(app.active_task()?, "ab");
        app.search_enter_char('c')?;
        assert_eq!(app.active_task()?, "abc");
        app.search_remove_char()?;
        assert_eq!(
            app.active_task()?,
            "abc",
            "should not move off of a search result if still a match"
        );
        Ok(())
    }

    #[test]
    fn test_search_scroll() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "ab".to_string(), "abc".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.enter_search()?;
        app.search_enter_char('b')?;
        assert_eq!(app.active_task()?, "ab");
        app.start_task("ab", OutputLogs::Full)?;
        assert_eq!(
            app.active_task()?,
            "ab",
            "starting the selected task keeps selection"
        );
        app.search_scroll(Direction::Down)?;
        assert_eq!(app.active_task()?, "abc");
        app.search_scroll(Direction::Down)?;
        assert_eq!(app.active_task()?, "ab", "wraps around to first match");
        app.search_scroll(Direction::Up)?;
        assert_eq!(app.active_task()?, "abc", "wraps around to last match");
        app.search_scroll(Direction::Up)?;
        assert_eq!(app.active_task()?, "ab");
        Ok(())
    }

    #[test]
    fn test_exit_search_restore_selection() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "abc".to_string(), "b".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.next();
        assert_eq!(app.active_task()?, "abc");
        app.enter_search()?;
        app.search_enter_char('b')?;
        assert_eq!(app.active_task()?, "abc");
        app.search_scroll(Direction::Down)?;
        assert_eq!(app.active_task()?, "b");
        app.exit_search(true);
        assert_eq!(app.active_task()?, "abc");
        Ok(())
    }

    #[test]
    fn test_exit_search_keep_selection() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "abc".to_string(), "b".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.next();
        assert_eq!(app.active_task()?, "abc");
        app.enter_search()?;
        app.search_enter_char('b')?;
        assert_eq!(app.active_task()?, "abc");
        app.search_scroll(Direction::Down)?;
        assert_eq!(app.active_task()?, "b");
        app.exit_search(false);
        assert_eq!(app.active_task()?, "b");
        Ok(())
    }

    #[test]
    fn test_select_update_task_removes_task() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "ab".to_string(), "abc".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.enter_search()?;
        app.search_enter_char('b')?;
        assert_eq!(app.active_task()?, "ab");
        // Remove selected task ab
        app.update_tasks(vec!["a".into(), "abc".into()])?;
        assert_eq!(app.active_task()?, "abc");
        Ok(())
    }

    #[test]
    fn test_select_restart_tasks_reorders_tasks() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec!["a".to_string(), "ab".to_string(), "abc".to_string()],
            PreferenceLoader::new(&repo_root),
            2048,
        );
        app.enter_search()?;
        app.search_enter_char('b')?;
        assert_eq!(app.active_task()?, "ab");
        app.start_task("ab", OutputLogs::Full)?;
        assert_eq!(app.active_task()?, "ab");
        // Restart ab
        app.restart_tasks(vec!["ab".into()])?;
        assert_eq!(app.active_task()?, "ab");
        Ok(())
    }

    #[test]
    fn test_search_lock() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec![
                "app-a".to_string(),
                "app-b".to_string(),
                "pkg-a".to_string(),
            ],
            PreferenceLoader::new(&repo_root),
            2048,
        );

        // Enter search mode and type query
        app.enter_search()?;
        assert!(matches!(app.section_focus, LayoutSections::Search { .. }));
        app.search_enter_char('a')?;
        app.search_enter_char('p')?;
        app.search_enter_char('p')?;
        assert_eq!(app.active_task()?, "app-a");

        // Lock the search
        app.lock_search();
        assert!(
            matches!(app.section_focus, LayoutSections::SearchLocked { .. }),
            "search should be locked"
        );

        // Verify we're still on the same task
        assert_eq!(app.active_task()?, "app-a");

        Ok(())
    }

    #[test]
    fn test_locked_search_navigation() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec![
                "app-a".to_string(),
                "app-b".to_string(),
                "pkg-a".to_string(),
                "pkg-b".to_string(),
            ],
            PreferenceLoader::new(&repo_root),
            2048,
        );

        // Enter search and lock with "app" query
        app.enter_search()?;
        app.search_enter_char('a')?;
        app.search_enter_char('p')?;
        app.search_enter_char('p')?;
        assert_eq!(app.active_task()?, "app-a");
        app.lock_search();

        // Navigate down should only go to next matching task (app-b)
        app.next();
        assert_eq!(
            app.active_task()?,
            "app-b",
            "should skip to next matching task"
        );

        // Navigate down again should wrap to first match
        app.next();
        assert_eq!(
            app.active_task()?,
            "app-a",
            "should wrap around to first matching task"
        );

        // Navigate up should wrap to last match
        app.previous();
        assert_eq!(
            app.active_task()?,
            "app-b",
            "should wrap to last matching task"
        );

        // Navigate up should go to previous match
        app.previous();
        assert_eq!(app.active_task()?, "app-a");

        Ok(())
    }

    #[test]
    fn test_locked_search_only_shows_matches() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec![
                "build-web".to_string(),
                "build-api".to_string(),
                "test-web".to_string(),
                "test-api".to_string(),
            ],
            PreferenceLoader::new(&repo_root),
            2048,
        );

        // Search for "build" - tasks are sorted alphabetically so build-api comes first
        app.enter_search()?;
        app.search_enter_char('b')?;
        app.search_enter_char('u')?;
        app.search_enter_char('i')?;
        app.search_enter_char('l')?;
        app.search_enter_char('d')?;
        assert_eq!(app.active_task()?, "build-api");
        app.lock_search();

        // Navigate through all tasks - should only hit "build" tasks
        let mut visited = vec![app.active_task()?.to_string()];
        app.next();
        visited.push(app.active_task()?.to_string());
        app.next();
        visited.push(app.active_task()?.to_string());

        assert_eq!(visited, vec!["build-api", "build-web", "build-api"]);
        assert!(
            !visited.contains(&"test-web".to_string()),
            "should not visit non-matching tasks"
        );
        assert!(
            !visited.contains(&"test-api".to_string()),
            "should not visit non-matching tasks"
        );

        Ok(())
    }

    #[test]
    fn test_exit_locked_search() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec![
                "app-a".to_string(),
                "app-b".to_string(),
                "pkg-a".to_string(),
            ],
            PreferenceLoader::new(&repo_root),
            2048,
        );

        // Enter search, lock it
        app.enter_search()?;
        app.search_enter_char('a')?;
        app.search_enter_char('p')?;
        app.search_enter_char('p')?;
        app.lock_search();
        assert!(matches!(
            app.section_focus,
            LayoutSections::SearchLocked { .. }
        ));

        // Exit should go back to normal task list
        app.exit_search(false);
        assert!(
            matches!(app.section_focus, LayoutSections::TaskList),
            "should exit to task list"
        );

        // Navigation should now go through all tasks
        app.next();
        app.next();
        app.next();
        // Starting from app-a, should cycle through app-b, pkg-a, back to app-a
        assert_eq!(app.active_task()?, "app-a");

        Ok(())
    }

    #[test]
    fn test_locked_search_with_single_match() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec![
                "app-a".to_string(),
                "app-b".to_string(),
                "unique".to_string(),
            ],
            PreferenceLoader::new(&repo_root),
            2048,
        );

        // Search for unique string
        app.enter_search()?;
        app.search_enter_char('u')?;
        app.search_enter_char('n')?;
        app.search_enter_char('i')?;
        app.search_enter_char('q')?;
        app.search_enter_char('u')?;
        app.search_enter_char('e')?;
        assert_eq!(app.active_task()?, "unique");
        app.lock_search();

        // Navigate should stay on the same task
        app.next();
        assert_eq!(app.active_task()?, "unique", "should stay on single match");
        app.previous();
        assert_eq!(app.active_task()?, "unique", "should stay on single match");

        Ok(())
    }

    #[test]
    fn test_locked_search_maintains_filter_after_task_changes() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec![
                "app-a".to_string(),
                "app-b".to_string(),
                "pkg-a".to_string(),
            ],
            PreferenceLoader::new(&repo_root),
            2048,
        );

        // Lock search for "app"
        app.enter_search()?;
        app.search_enter_char('a')?;
        app.search_enter_char('p')?;
        app.search_enter_char('p')?;
        app.lock_search();
        assert_eq!(app.active_task()?, "app-a");

        // Start a task
        app.start_task("app-a", OutputLogs::Full)?;

        // Navigation should still only show "app" tasks
        app.next();
        assert_eq!(
            app.active_task()?,
            "app-b",
            "filter should persist after task changes"
        );

        Ok(())
    }

    #[test]
    fn test_search_wrap_around_empty_results() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec![
                "app-a".to_string(),
                "app-b".to_string(),
                "pkg-a".to_string(),
            ],
            PreferenceLoader::new(&repo_root),
            2048,
        );

        // Search for something that doesn't match anything
        app.enter_search()?;
        app.search_enter_char('x')?;
        app.search_enter_char('y')?;
        app.search_enter_char('z')?;

        // Try to scroll - should not panic or change selection
        let initial_task = app.active_task()?.to_string();
        app.search_scroll(Direction::Down)?;
        assert_eq!(app.active_task()?, initial_task);
        app.search_scroll(Direction::Up)?;
        assert_eq!(app.active_task()?, initial_task);

        Ok(())
    }

    #[test]
    fn test_search_shows_hidden_task_list() -> Result<(), Error> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())
            .expect("Failed to create AbsoluteSystemPathBuf");

        let mut app: App<()> = App::new(
            100,
            100,
            vec![
                "app-a".to_string(),
                "app-b".to_string(),
                "pkg-a".to_string(),
            ],
            PreferenceLoader::new(&repo_root),
            2048,
        );

        // Hide the task list
        app.preferences.set_is_task_list_visible(Some(false));
        assert!(!app.preferences.is_task_list_visible());

        // Enter search mode
        app.enter_search()?;

        // Task list should now be visible
        assert!(
            app.preferences.is_task_list_visible(),
            "task list should be visible after entering search"
        );

        Ok(())
    }
}
