use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::frame::{
    Attributes, Cell, Cursor, DEFAULT_BACKGROUND, DEFAULT_FOREGROUND, FORMAT_VERSION, Frame,
};
use crate::recording;
use crate::session::{Session, SessionLaunch, SessionState, SessionStatus};
use crate::shot::{Options, Shot};
use crate::terminal_core::InputModes;
use crate::terminal_theme::TerminalTheme;

pub type PaneId = u32;
pub type WindowId = u32;
type PaneRevisions = Vec<(PaneId, u64)>;
type CapturedInput = Vec<(recording::InputOrigin, Vec<u8>)>;

const DEFAULT_WINDOW_NAME: &str = "main";

const VERTICAL_DIVIDER: &str = "│";
const HORIZONTAL_DIVIDER: &str = "─";
const PREFIX: u8 = 0x02;
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
const SGR_MOUSE_PREFIX: &[u8] = b"\x1b[<";
const PASTE_CHUNK_BYTES: usize = 64 * 1024;
const MAX_SGR_MOUSE_BYTES: usize = 64;
const MAX_ATTACHMENT_INPUTS_PER_TICK: usize = 64;
const MAX_ATTACHMENT_INPUT_BYTES_PER_TICK: usize = 64 * 1024;
const MAX_ATTACHMENT_ACTIONS_PER_TICK: usize = 1024;
const MAX_WORKSPACE_PANES: usize = 64;
const TAB_STRIP_ROWS: u16 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TabPosition {
    Top,
    #[default]
    Bottom,
}

impl TabPosition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ActivityKind {
    Output,
    Bell,
    Exit,
}

impl ActivityKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Output => "output",
            Self::Bell => "bell",
            Self::Exit => "exit",
        }
    }

    fn badge(self) -> char {
        match self {
            Self::Output => '+',
            Self::Bell => '!',
            Self::Exit => 'x',
        }
    }
}

pub(crate) struct Workspace {
    name: String,
    windows: Vec<Window>,
    active_window: WindowId,
    previous_window: Option<WindowId>,
    next_window_id: WindowId,
    next_pane_id: PaneId,
    launch: SessionLaunch,
    cols: u16,
    rows: u16,
    chrome_generation: u64,
    recording: Option<WorkspaceRecording>,
    pending_input: CapturedInput,
    tab_position: TabPosition,
}

struct WorkspaceRecording {
    writer: recording::Writer,
    window: Option<(WindowId, u64, u64)>,
    revisions: PaneRevisions,
}

pub(crate) struct Window {
    id: WindowId,
    name: String,
    workspace: String,
    panes: Vec<Pane>,
    active: Option<PaneId>,
    layout: Option<LayoutNode>,
    applied: AppliedLayout,
    cols: u16,
    rows: u16,
    cwd: PathBuf,
    shell: Vec<String>,
    options: Options,
    theme: TerminalTheme,
    paste: Option<(PaneId, bool)>,
    zoomed: Option<PaneId>,
    cached_frame: Option<(PaneRevisions, Frame)>,
    frame_generation: u64,
    activity_kinds: BTreeSet<ActivityKind>,
    capture_input: bool,
}

struct WindowIdentity {
    id: WindowId,
    name: String,
    workspace: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WindowStatus {
    pub index: usize,
    pub name: String,
    pub active: bool,
    pub pane_count: usize,
    pub active_pane: Option<PaneId>,
    #[serde(default)]
    pub zoomed_pane: Option<PaneId>,
    #[serde(default)]
    pub activity: bool,
    #[serde(default)]
    pub activity_kinds: Vec<ActivityKind>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkspaceContext {
    pub session: String,
    pub workspace: String,
    pub window_id: WindowId,
    pub window: String,
    pub window_index: usize,
    pub pane: PaneId,
    pub window_active: bool,
    pub pane_active: bool,
    pub tab_position: TabPosition,
}

struct Pane {
    id: PaneId,
    session: Session,
}

fn stop_pane(mut pane: Pane, pending_input: &mut CapturedInput) -> Result<()> {
    pending_input.extend(pane.session.take_captured_input());
    let result = pane.session.stop();
    pending_input.extend(pane.session.take_captured_input());
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitAxis {
    Columns,
    Rows,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LayoutNode {
    Leaf(PaneId),
    Split {
        axis: SplitAxis,
        offset: i16,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

impl LayoutNode {
    fn contains(&self, pane: PaneId) -> bool {
        match self {
            Self::Leaf(id) => *id == pane,
            Self::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
    }

    fn first_leaf(&self) -> PaneId {
        match self {
            Self::Leaf(id) => *id,
            Self::Split { first, .. } => first.first_leaf(),
        }
    }

    fn split_leaf(&mut self, target: PaneId, axis: SplitAxis, pane: PaneId) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                *self = Self::Split {
                    axis,
                    offset: 0,
                    first: Box::new(Self::Leaf(target)),
                    second: Box::new(Self::Leaf(pane)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split_leaf(target, axis, pane) || second.split_leaf(target, axis, pane)
            }
        }
    }

    fn resize_leaf(&mut self, target: PaneId, direction: Direction, amount: i16) -> bool {
        match self {
            Self::Leaf(_) => false,
            Self::Split {
                axis,
                offset,
                first,
                second,
            } => {
                if first.contains(target) {
                    if first.resize_leaf(target, direction, amount) {
                        return true;
                    }
                    if matches!(
                        (*axis, direction),
                        (SplitAxis::Columns, Direction::Right) | (SplitAxis::Rows, Direction::Down)
                    ) {
                        *offset = offset.saturating_add(amount);
                        return true;
                    }
                } else if second.contains(target) {
                    if second.resize_leaf(target, direction, amount) {
                        return true;
                    }
                    if matches!(
                        (*axis, direction),
                        (SplitAxis::Columns, Direction::Left) | (SplitAxis::Rows, Direction::Up)
                    ) {
                        *offset = offset.saturating_sub(amount);
                        return true;
                    }
                }
                false
            }
        }
    }

    fn remove_leaf(self, target: PaneId) -> (Option<Self>, bool) {
        match self {
            Self::Leaf(id) if id == target => (None, true),
            Self::Leaf(_) => (Some(self), false),
            Self::Split {
                axis,
                offset,
                first,
                second,
            } => {
                if first.contains(target) {
                    let (first, removed) = first.remove_leaf(target);
                    let tree = match first {
                        Some(first) => Some(Self::Split {
                            axis,
                            offset,
                            first: Box::new(first),
                            second,
                        }),
                        None => Some(*second),
                    };
                    (tree, removed)
                } else if second.contains(target) {
                    let (second, removed) = second.remove_leaf(target);
                    let tree = match second {
                        Some(second) => Some(Self::Split {
                            axis,
                            offset,
                            first,
                            second: Box::new(second),
                        }),
                        None => Some(*first),
                    };
                    (tree, removed)
                } else {
                    (
                        Some(Self::Split {
                            axis,
                            offset,
                            first,
                            second,
                        }),
                        false,
                    )
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }

    fn unavailable(self) -> &'static str {
        match self {
            Self::Left => "no pane to the left",
            Self::Right => "no pane to the right",
            Self::Up => "no pane above",
            Self::Down => "no pane below",
        }
    }
}

fn directional_score(
    current: PaneRect,
    candidate: PaneRect,
    pane: PaneId,
    direction: Direction,
) -> Option<(bool, u16, u16, PaneId)> {
    let current_right = current.x.saturating_add(current.cols);
    let candidate_right = candidate.x.saturating_add(candidate.cols);
    let current_bottom = current.y.saturating_add(current.rows);
    let candidate_bottom = candidate.y.saturating_add(candidate.rows);
    let current_x = current.x.saturating_mul(2).saturating_add(current.cols);
    let candidate_x = candidate.x.saturating_mul(2).saturating_add(candidate.cols);
    let current_y = current.y.saturating_mul(2).saturating_add(current.rows);
    let candidate_y = candidate.y.saturating_mul(2).saturating_add(candidate.rows);
    let horizontal_overlap = current.x < candidate_right && candidate.x < current_right;
    let vertical_overlap = current.y < candidate_bottom && candidate.y < current_bottom;
    match direction {
        Direction::Left if candidate_right <= current.x => Some((
            !vertical_overlap,
            current.x - candidate_right,
            current_y.abs_diff(candidate_y),
            pane,
        )),
        Direction::Right if candidate.x >= current_right => Some((
            !vertical_overlap,
            candidate.x - current_right,
            current_y.abs_diff(candidate_y),
            pane,
        )),
        Direction::Up if candidate_bottom <= current.y => Some((
            !horizontal_overlap,
            current.y - candidate_bottom,
            current_x.abs_diff(candidate_x),
            pane,
        )),
        Direction::Down if candidate.y >= current_bottom => Some((
            !horizontal_overlap,
            candidate.y - current_bottom,
            current_x.abs_diff(candidate_x),
            pane,
        )),
        _ => None,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PaneStatus {
    pub id: PaneId,
    pub active: bool,
    #[serde(default = "default_true")]
    pub visible: bool,
    pub state: SessionState,
    #[serde(default)]
    pub x: u16,
    #[serde(default)]
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
    #[serde(default)]
    pub title: String,
    pub command: Vec<String>,
    pub cwd: PathBuf,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PaneRect {
    x: u16,
    y: u16,
    cols: u16,
    rows: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlacedPane {
    id: PaneId,
    rect: PaneRect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Divider {
    axis: SplitAxis,
    x: u16,
    y: u16,
    len: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkspaceGeometry {
    panes: Vec<PlacedPane>,
    dividers: Vec<Divider>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AppliedLayout {
    Ready(WorkspaceGeometry),
    Constrained(WorkspaceGeometry),
}

impl AppliedLayout {
    fn geometry(&self) -> &WorkspaceGeometry {
        match self {
            Self::Ready(geometry) | Self::Constrained(geometry) => geometry,
        }
    }

    fn geometry_mut(&mut self) -> &mut WorkspaceGeometry {
        match self {
            Self::Ready(geometry) | Self::Constrained(geometry) => geometry,
        }
    }

    fn is_constrained(&self) -> bool {
        matches!(self, Self::Constrained(_))
    }
}

impl Workspace {
    #[cfg(test)]
    pub(crate) fn start(
        command: &[String],
        cwd: Option<&Path>,
        record: Option<&Path>,
        options: &Options,
    ) -> Result<Self> {
        Self::start_with_theme(
            command,
            cwd,
            record,
            options,
            TerminalTheme::default(),
            TabPosition::Bottom,
        )
    }

    #[cfg(test)]
    pub(crate) fn start_with_theme(
        command: &[String],
        cwd: Option<&Path>,
        record: Option<&Path>,
        options: &Options,
        theme: TerminalTheme,
        tab_position: TabPosition,
    ) -> Result<Self> {
        Self::start_named_with_theme(
            "workspace",
            command,
            cwd,
            record,
            options,
            theme,
            tab_position,
        )
    }

    pub(crate) fn start_named_with_theme(
        name: &str,
        command: &[String],
        cwd: Option<&Path>,
        record: Option<&Path>,
        options: &Options,
        theme: TerminalTheme,
        tab_position: TabPosition,
    ) -> Result<Self> {
        let mut content_options = options.clone();
        content_options.rows = content_rows(options.rows)?;
        let started = Instant::now();
        let mut main = Window::start_with_theme(
            WindowIdentity {
                id: 0,
                name: DEFAULT_WINDOW_NAME.to_owned(),
                workspace: name.to_owned(),
            },
            0,
            command,
            cwd,
            &content_options,
            theme,
            record.is_some(),
        )?;
        let mut launch = main.panes[0].session.status()?.launch;
        launch.rows = options.rows;
        launch.record = record.map(Path::to_path_buf);
        let recording = record
            .map(|path| {
                recording::Writer::new(
                    path,
                    started,
                    options.cols,
                    options.rows,
                    options.cell_width,
                    options.cell_height,
                )
                .map(|writer| WorkspaceRecording {
                    writer,
                    window: None,
                    revisions: Vec::new(),
                })
            })
            .transpose()?;
        Ok(Self {
            name: name.to_owned(),
            windows: vec![main],
            active_window: 0,
            previous_window: None,
            next_window_id: 1,
            next_pane_id: 1,
            launch,
            cols: options.cols,
            rows: options.rows,
            chrome_generation: 0,
            recording,
            pending_input: Vec::new(),
            tab_position,
        })
    }

    fn active_window_index(&self) -> Option<usize> {
        self.windows
            .iter()
            .position(|window| window.id == self.active_window)
    }

    fn active_window_name(&self) -> Option<&str> {
        self.active_window_index()
            .map(|index| self.windows[index].name.as_str())
    }

    fn window_index(&self, name: &str) -> Result<usize> {
        self.windows
            .iter()
            .position(|window| window.name == name)
            .ok_or_else(|| anyhow::anyhow!("workspace has no window {name:?}"))
    }

    fn window_id(&self, name: &str) -> Result<WindowId> {
        let index = self.window_index(name)?;
        Ok(self.windows[index].id)
    }

    fn window_index_or_active(&self, name: Option<&str>) -> Result<usize> {
        match name {
            Some(name) => self.window_index(name),
            None => self
                .active_window_index()
                .context("workspace has no active window"),
        }
    }

    fn selected_window(&self) -> Result<&Window> {
        let index = self.window_index_or_active(None)?;
        Ok(&self.windows[index])
    }

    fn selected_window_mut(&mut self) -> Result<&mut Window> {
        let index = self.window_index_or_active(None)?;
        Ok(&mut self.windows[index])
    }

    fn pane_window_index(&self, pane: PaneId) -> Option<usize> {
        self.windows
            .iter()
            .position(|window| window.pane_index(pane).is_some())
    }

    pub(crate) fn windows(&self) -> Vec<WindowStatus> {
        self.windows
            .iter()
            .enumerate()
            .map(|(index, window)| WindowStatus {
                index,
                name: window.name.clone(),
                active: window.id == self.active_window,
                pane_count: window.panes.len(),
                active_pane: window.active,
                zoomed_pane: window.zoomed,
                activity: !window.activity_kinds.is_empty(),
                activity_kinds: window.activity_kinds.iter().copied().collect(),
                cols: self.cols,
                rows: self.rows,
            })
            .collect()
    }

    pub(crate) fn context(&self, pane: Option<PaneId>) -> Result<WorkspaceContext> {
        let pane = pane
            .or_else(|| self.selected_window().ok().and_then(|window| window.active))
            .context("workspace has no active pane")?;
        let window_index = self
            .pane_window_index(pane)
            .ok_or_else(|| anyhow::anyhow!("workspace has no pane {pane}"))?;
        let window = &self.windows[window_index];
        Ok(WorkspaceContext {
            session: self.name.clone(),
            workspace: self.name.clone(),
            window_id: window.id,
            window: window.name.clone(),
            window_index,
            pane,
            window_active: window.id == self.active_window,
            pane_active: window.active == Some(pane),
            tab_position: self.tab_position,
        })
    }

    pub(crate) fn set_tab_position(&mut self, position: TabPosition) {
        if self.tab_position != position {
            self.tab_position = position;
            self.chrome_generation = self.chrome_generation.wrapping_add(1);
        }
    }

    pub(crate) fn move_window(&mut self, name: &str, target: usize) -> Result<()> {
        let index = self.window_index(name)?;
        if target >= self.windows.len() {
            bail!(
                "window index {target} is out of range for {} windows",
                self.windows.len()
            );
        }
        if index == target {
            return Ok(());
        }
        let window = self.windows.remove(index);
        self.windows.insert(target, window);
        self.chrome_generation = self.chrome_generation.wrapping_add(1);
        Ok(())
    }

    fn move_active_window(&mut self, offset: isize) -> Result<bool> {
        let current = self
            .active_window_index()
            .context("workspace has no active window")?;
        let target = isize::try_from(current).unwrap_or(0) + offset;
        if target < 0 || target >= isize::try_from(self.windows.len()).unwrap_or(isize::MAX) {
            return Ok(false);
        }
        let target = usize::try_from(target).unwrap_or(current);
        let name = self.windows[current].name.clone();
        self.move_window(&name, target)?;
        Ok(true)
    }

    pub(crate) fn panes_in(&mut self, name: Option<&str>) -> Result<Vec<PaneStatus>> {
        let index = self.window_index_or_active(name)?;
        self.pane_statuses(index)
    }

    fn pane_statuses(&mut self, index: usize) -> Result<Vec<PaneStatus>> {
        let mut panes = self.windows[index].panes()?;
        if self.tab_position == TabPosition::Top {
            for pane in &mut panes {
                pane.y = pane.y.saturating_add(TAB_STRIP_ROWS);
            }
        }
        Ok(panes)
    }

    #[cfg(test)]
    pub(crate) fn set_grid_in(
        &mut self,
        name: Option<&str>,
        columns: u16,
        rows: u16,
    ) -> Result<Vec<PaneStatus>> {
        self.set_grid_in_with_command(name, columns, rows, None)
    }

    pub(crate) fn set_grid_in_with_command(
        &mut self,
        name: Option<&str>,
        columns: u16,
        rows: u16,
        command: Option<&[String]>,
    ) -> Result<Vec<PaneStatus>> {
        let index = self.window_index_or_active(name)?;
        let current = self.pane_count();
        let desired = usize::from(columns) * usize::from(rows);
        let added = desired.saturating_sub(self.windows[index].panes.len());
        if current.saturating_add(added) > MAX_WORKSPACE_PANES {
            bail!("workspace supports at most {MAX_WORKSPACE_PANES} panes");
        }
        let first_new_id = self.next_pane_id;
        let next_pane_id = first_new_id
            .checked_add(u32::try_from(added).unwrap_or(u32::MAX))
            .context("workspace exhausted stable pane ids")?;
        self.windows[index].set_grid(
            columns,
            rows,
            first_new_id,
            command,
            &mut self.pending_input,
        )?;
        self.next_pane_id = next_pane_id;
        self.chrome_generation = self.chrome_generation.wrapping_add(1);
        self.pane_statuses(index)
    }

    pub(crate) fn send_all_in(
        &mut self,
        name: &str,
        input: &[Vec<u8>],
        pace: Duration,
        tick: impl FnMut(&mut Self) -> Result<bool>,
    ) -> Result<()> {
        let index = self.window_index(name)?;
        let pane = self.windows[index]
            .active
            .context("window has no active pane")?;
        self.send_all(Some(pane), input, pace, tick)
    }

    pub(crate) fn wait_for_text_in(
        &mut self,
        name: &str,
        text: &str,
        timeout: Duration,
        tick: impl FnMut(&mut Self) -> Result<bool>,
    ) -> Result<()> {
        let index = self.window_index(name)?;
        let pane = self.windows[index]
            .active
            .context("window has no active pane")?;
        self.wait_for_text(Some(pane), text, timeout, tick)
    }

    pub(crate) fn capture_window(
        &mut self,
        name: &str,
        settle: Duration,
        deadline: Duration,
        tick: impl FnMut(&mut Self) -> Result<bool>,
    ) -> Result<Shot> {
        let id = self.window_id(name)?;
        self.capture_target(id, None, settle, deadline, tick)
    }

    pub(crate) fn logs_in(&mut self, name: &str, ansi: bool) -> Result<Vec<u8>> {
        let index = self.window_index(name)?;
        self.windows[index].active_logs(ansi)
    }

    pub(crate) fn semantic_snapshot_in(
        &mut self,
        name: Option<&str>,
        pane: Option<PaneId>,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        if name.is_some() && pane.is_some() {
            bail!("window and pane cannot be combined; pane ids are already globally stable");
        }
        let index = pane
            .and_then(|pane| self.pane_window_index(pane))
            .map_or_else(|| self.window_index_or_active(name), Ok)?;
        let pane = pane
            .or(self.windows[index].active)
            .context("workspace target has no active pane")?;
        self.windows[index]
            .panes
            .iter_mut()
            .find(|candidate| candidate.id == pane)
            .with_context(|| format!("workspace has no pane {pane}"))?
            .session
            .semantic_snapshot(timeout)
    }

    pub(crate) fn create_window(
        &mut self,
        name: Option<&str>,
        command: &[String],
        cwd: Option<&Path>,
    ) -> Result<WindowId> {
        if self.pane_count() >= MAX_WORKSPACE_PANES {
            bail!("workspace supports at most {MAX_WORKSPACE_PANES} panes");
        }
        let id = self.next_window_id;
        let next_window_id = id
            .checked_add(1)
            .context("workspace exhausted stable window ids")?;
        let name = name
            .map(str::to_owned)
            .unwrap_or_else(|| format!("window-{id}"));
        validate_window_name(&name)?;
        if self.windows.iter().any(|window| window.name == name) {
            bail!("workspace already has a window named {name:?}");
        }
        let active = self.window_index_or_active(None)?;
        let source_cwd = self.windows[active].cwd.clone();
        let source_options = self.windows[active].options.clone();
        let source_theme = self.windows[active].theme;
        let pane_id = self.next_pane_id;
        let next_pane_id = pane_id
            .checked_add(1)
            .context("workspace exhausted stable pane ids")?;
        let window = Window::start_with_theme(
            WindowIdentity {
                id,
                name,
                workspace: self.name.clone(),
            },
            pane_id,
            command,
            cwd.or(Some(source_cwd.as_path())),
            &source_options,
            source_theme,
            self.recording.is_some(),
        )?;
        self.windows.push(window);
        self.next_window_id = next_window_id;
        self.next_pane_id = next_pane_id;
        self.chrome_generation = self.chrome_generation.wrapping_add(1);
        self.select_window_id(id)?;
        Ok(id)
    }

    pub(crate) fn rename_window(&mut self, name: &str, new_name: &str) -> Result<()> {
        if name == new_name {
            self.window_index(name)?;
            return Ok(());
        }
        validate_window_name(new_name)?;
        if self.windows.iter().any(|window| window.name == new_name) {
            bail!("workspace already has a window named {new_name:?}");
        }
        let index = self.window_index(name)?;
        self.windows[index].name = new_name.to_owned();
        self.chrome_generation = self.chrome_generation.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn select_window(&mut self, name: &str) -> Result<()> {
        self.select_window_id(self.window_id(name)?)
    }

    fn select_window_index(&mut self, index: usize) -> Result<bool> {
        let Some(window) = self.windows.get(index) else {
            return Ok(false);
        };
        let id = window.id;
        self.select_window_id(id)?;
        Ok(true)
    }

    fn select_window_id(&mut self, id: WindowId) -> Result<()> {
        if self.active_window == id {
            return Ok(());
        }
        let next = self
            .windows
            .iter()
            .position(|window| window.id == id)
            .ok_or_else(|| anyhow::anyhow!("workspace has no window {id}"))?;
        if let Some(current) = self.active_window_index() {
            self.previous_window = Some(self.windows[current].id);
            self.windows[current].cancel_paste();
            let active = self.windows[current].active;
            let _ = self.windows[current].send_focus(active, false);
        }
        self.active_window = id;
        self.windows[next].clear_activity();
        self.chrome_generation = self.chrome_generation.wrapping_add(1);
        let active = self.windows[next].active;
        let _ = self.windows[next].send_focus(active, true);
        Ok(())
    }

    fn select_relative_window(&mut self, offset: isize) -> Result<bool> {
        if self.windows.len() < 2 {
            return Ok(false);
        }
        let current = self
            .active_window_index()
            .context("workspace has no active window")?;
        let len = isize::try_from(self.windows.len()).unwrap_or(isize::MAX);
        let next = (isize::try_from(current).unwrap_or(0) + offset).rem_euclid(len);
        self.select_window_index(usize::try_from(next).unwrap_or(0))
    }

    fn select_previous_window(&mut self) -> Result<bool> {
        let Some(previous) = self.previous_window else {
            return Ok(false);
        };
        if !self.windows.iter().any(|window| window.id == previous) {
            self.previous_window = None;
            return Ok(false);
        }
        self.select_window_id(previous)?;
        Ok(true)
    }

    pub(crate) fn close_window(&mut self, name: &str) -> Result<()> {
        let index = self.window_index(name)?;
        self.close_window_index(index)
    }

    fn close_window_index(&mut self, index: usize) -> Result<()> {
        let closing_id = self.windows[index].id;
        let closing_active = closing_id == self.active_window;
        let previous = self.previous_window.filter(|previous| {
            *previous != closing_id && self.windows.iter().any(|window| window.id == *previous)
        });
        if closing_active {
            let active = self.windows[index].active;
            let _ = self.windows[index].send_focus(active, false);
        }
        let mut window = self.windows.remove(index);
        window.stop(&mut self.pending_input);
        self.flush_input(None)?;
        self.chrome_generation = self.chrome_generation.wrapping_add(1);
        if self.windows.is_empty() {
            self.previous_window = None;
            return Ok(());
        }
        if self.previous_window == Some(window.id) {
            self.previous_window = None;
        }
        if closing_active {
            let next = previous
                .and_then(|previous| self.windows.iter().position(|window| window.id == previous))
                .unwrap_or_else(|| index.min(self.windows.len() - 1));
            self.active_window = self.windows[next].id;
            self.previous_window = None;
            let active = self.windows[next].active;
            let _ = self.windows[next].send_focus(active, true);
        }
        Ok(())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    fn all_exits_observed(&self) -> bool {
        !self.windows.is_empty()
            && self
                .windows
                .iter()
                .all(|window| window.all_exits_observed())
    }

    pub(crate) fn set_theme(&mut self, theme: TerminalTheme) -> Result<()> {
        let Some(first) = self.windows.first() else {
            return Ok(());
        };
        let previous = first.theme;
        if previous == theme {
            return Ok(());
        }
        for index in 0..self.windows.len() {
            if let Err(error) = self.windows[index].set_theme(theme) {
                for window in &mut self.windows[..index] {
                    let _ = window.set_theme(previous);
                }
                return Err(error);
            }
        }
        Ok(())
    }

    pub(crate) fn pump(&mut self) -> Result<()> {
        for index in 0..self.windows.len() {
            let changed = self.windows[index].pump(&mut self.pending_input)?;
            if changed
                && self.windows[index].id != self.active_window
                && self.windows[index].mark_activity(ActivityKind::Output)
            {
                self.chrome_generation = self.chrome_generation.wrapping_add(1);
            }
        }
        self.flush_input(Some(recording::InputOrigin::Client))?;
        self.record_frame()?;
        self.flush_input(Some(recording::InputOrigin::Host))?;
        Ok(())
    }

    fn flush_input(&mut self, only: Option<recording::InputOrigin>) -> Result<()> {
        let Some(recording) = &mut self.recording else {
            self.pending_input.clear();
            return Ok(());
        };
        let mut remaining = Vec::new();
        let mut pending = std::mem::take(&mut self.pending_input).into_iter();
        while let Some((origin, bytes)) = pending.next() {
            if only.is_none_or(|only| only == origin) {
                if let Err(error) = recording.writer.input(origin, &bytes) {
                    remaining.push((origin, bytes));
                    remaining.extend(pending);
                    self.pending_input = remaining;
                    return Err(error);
                }
            } else {
                remaining.push((origin, bytes));
            }
        }
        self.pending_input = remaining;
        Ok(())
    }

    fn record_frame(&mut self) -> Result<()> {
        if self.recording.is_none() || self.windows.is_empty() {
            return Ok(());
        }
        let mut revisions = Vec::new();
        let window = self.active_frame_key(&mut revisions)?;
        if self.recording.as_ref().is_some_and(|recording| {
            recording.window == Some(window) && recording.revisions == revisions
        }) {
            return Ok(());
        }
        let frame = self.frame_with_revisions(&revisions)?;
        let bytes = frame_ansi(&frame)?;
        let recording = self.recording.as_mut().expect("recording checked above");
        recording.writer.output_now(&bytes)?;
        recording.window = Some(window);
        recording.revisions = revisions;
        Ok(())
    }

    pub(crate) fn observe_exits(&mut self) -> Result<bool> {
        let mut exited = false;
        for window in &mut self.windows {
            let changed = window.observe_exits()?;
            if changed
                && window.id != self.active_window
                && window.mark_activity(ActivityKind::Exit)
            {
                self.chrome_generation = self.chrome_generation.wrapping_add(1);
            }
            exited |= changed;
        }
        Ok(exited)
    }

    pub(crate) fn remove_observed_exits(&mut self) -> Result<bool> {
        let mut removed = false;
        for window in &mut self.windows {
            removed |= window
                .remove_observed_exits(window.id == self.active_window, &mut self.pending_input)?;
        }
        self.flush_input(None)?;
        let empty = self
            .windows
            .iter()
            .enumerate()
            .filter_map(|(index, window)| window.is_empty().then_some(index))
            .collect::<Vec<_>>();
        for index in empty.into_iter().rev() {
            self.close_window_index(index)?;
        }
        Ok(removed)
    }

    pub(crate) fn send(&mut self, pane: Option<PaneId>, input: &[u8]) -> Result<()> {
        match pane {
            Some(pane) => {
                let window = self
                    .pane_window_index(pane)
                    .ok_or_else(|| anyhow::anyhow!("workspace has no pane {pane}"))?;
                self.windows[window].send(Some(pane), input)
            }
            None => self.selected_window_mut()?.send(None, input),
        }
    }

    fn pane_count(&self) -> usize {
        self.windows.iter().map(|window| window.panes.len()).sum()
    }

    fn split(&mut self, axis: SplitAxis) -> Result<()> {
        if self.pane_count() >= MAX_WORKSPACE_PANES {
            bail!("workspace supports at most {MAX_WORKSPACE_PANES} panes");
        }
        let pane = self.next_pane_id;
        let next_pane_id = pane
            .checked_add(1)
            .context("workspace exhausted stable pane ids")?;
        let window = self.window_index_or_active(None)?;
        let result = self.windows[window].split(axis, pane, &mut self.pending_input);
        if result.is_ok() || self.pane_window_index(pane).is_some() {
            self.next_pane_id = next_pane_id;
            self.chrome_generation = self.chrome_generation.wrapping_add(1);
        }
        result
    }

    pub(crate) fn focus_pane(&mut self, pane: PaneId) -> Result<()> {
        let window = self
            .pane_window_index(pane)
            .ok_or_else(|| anyhow::anyhow!("workspace has no pane {pane}"))?;
        let window_id = self.windows[window].id;
        self.select_window_id(window_id)?;
        self.selected_window_mut()?.focus_pane(pane)
    }

    pub(crate) fn close_pane(&mut self, pane: PaneId) -> Result<()> {
        let window = self
            .pane_window_index(pane)
            .ok_or_else(|| anyhow::anyhow!("workspace has no pane {pane}"))?;
        let focused = self.windows[window].id == self.active_window;
        self.windows[window].close_pane(pane, focused, &mut self.pending_input)?;
        self.flush_input(None)?;
        if self.windows[window].is_empty() {
            self.close_window_index(window)?;
        } else {
            self.chrome_generation = self.chrome_generation.wrapping_add(1);
        }
        Ok(())
    }

    pub(crate) fn resize_pane(
        &mut self,
        pane: PaneId,
        direction: Direction,
        cells: u16,
    ) -> Result<Vec<PaneStatus>> {
        if cells == 0 {
            bail!("pane resize must change at least one cell");
        }
        let window = self
            .pane_window_index(pane)
            .ok_or_else(|| anyhow::anyhow!("workspace has no pane {pane}"))?;
        self.windows[window].resize_pane(pane, direction, cells)?;
        self.pane_statuses(window)
    }

    pub(crate) fn toggle_zoom_pane(&mut self, pane: PaneId) -> Result<Vec<PaneStatus>> {
        let window = self
            .pane_window_index(pane)
            .ok_or_else(|| anyhow::anyhow!("workspace has no pane {pane}"))?;
        self.windows[window].resolve_pane(Some(pane))?;
        if self.windows[window].panes.len() < 2 {
            bail!("window needs at least two panes to zoom");
        }
        self.windows[window].toggle_zoom(pane)?;
        self.chrome_generation = self.chrome_generation.wrapping_add(1);
        self.focus_pane(pane)?;
        self.panes_in(None)
    }

    pub(crate) fn move_pane(
        &mut self,
        pane: PaneId,
        target_window: &str,
        vertical: bool,
    ) -> Result<()> {
        let source = self
            .pane_window_index(pane)
            .ok_or_else(|| anyhow::anyhow!("workspace has no pane {pane}"))?;
        let target = self.window_index(target_window)?;
        if source == target {
            bail!("pane {pane} already belongs to window {target_window:?}");
        }
        let target_pane = self.windows[target]
            .active
            .context("target window has no active pane")?;
        let axis = if vertical {
            SplitAxis::Rows
        } else {
            SplitAxis::Columns
        };

        let old_source_layout = self.windows[source].layout.clone();
        let old_source_applied = self.windows[source].applied.clone();
        let old_source_active = self.windows[source].active;
        let old_source_zoomed = self.windows[source].zoomed;
        let old_target_layout = self.windows[target].layout.clone();
        let old_target_applied = self.windows[target].applied.clone();
        let old_target_zoomed = self.windows[target].zoomed;

        let (source_layout, removed) = old_source_layout
            .clone()
            .context("source window has no layout")?
            .remove_leaf(pane);
        if !removed {
            bail!("source window layout has no pane {pane}");
        }
        let source_geometry = source_layout
            .as_ref()
            .map(|layout| geometry(layout, self.windows[source].cols, self.windows[source].rows))
            .transpose()?;
        let mut target_layout = old_target_layout
            .clone()
            .context("target window has no layout")?;
        if !target_layout.split_leaf(target_pane, axis, pane) {
            bail!("target window layout has no pane {target_pane}");
        }
        let target_geometry = geometry(
            &target_layout,
            self.windows[target].cols,
            self.windows[target].rows,
        )?;

        self.windows[source].clear_zoom()?;
        if let Err(error) = self.windows[target].clear_zoom() {
            self.windows[source].zoomed = old_source_zoomed;
            let _ = self.windows[source].refresh_layout();
            return Err(error);
        }

        let source_pane = self.windows[source]
            .pane_index(pane)
            .context("source window has no pane")?;
        if self.windows[source]
            .paste
            .is_some_and(|(target, _)| target == pane)
        {
            self.windows[source].cancel_paste();
        }
        if self.windows[source].id == self.active_window && old_source_active == Some(pane) {
            let _ = self.windows[source].send_focus(Some(pane), false);
        }
        let moved = self.windows[source].panes.remove(source_pane);
        self.windows[source].layout = source_layout;
        self.windows[source].active = if old_source_active == Some(pane) {
            self.windows[source].first_layout_pane()
        } else {
            old_source_active
        };
        let source_result = match source_geometry {
            Some(geometry) => self.windows[source].apply_geometry(geometry),
            None => {
                self.windows[source].applied = AppliedLayout::Ready(WorkspaceGeometry {
                    panes: Vec::new(),
                    dividers: Vec::new(),
                });
                self.windows[source].invalidate_frame();
                Ok(())
            }
        };
        if let Err(error) = source_result {
            self.windows[source].panes.insert(source_pane, moved);
            self.windows[source].layout = old_source_layout;
            let _ = self.windows[source].apply_geometry(old_source_applied.geometry().clone());
            self.windows[source].applied = old_source_applied;
            self.windows[source].active = old_source_active;
            self.windows[source].zoomed = old_source_zoomed;
            self.windows[source].invalidate_frame();
            self.windows[target].zoomed = old_target_zoomed;
            let _ = self.windows[target].refresh_layout();
            if self.windows[source].id == self.active_window && old_source_active == Some(pane) {
                let _ = self.windows[source].send_focus(Some(pane), true);
            }
            return Err(error);
        }

        self.windows[target].panes.push(moved);
        self.windows[target].layout = Some(target_layout);
        if let Err(error) = self.windows[target].apply_geometry(target_geometry) {
            let moved = self.windows[target]
                .panes
                .pop()
                .context("moved pane disappeared during rollback")?;
            self.windows[target].layout = old_target_layout;
            let _ = self.windows[target].apply_geometry(old_target_applied.geometry().clone());
            self.windows[target].applied = old_target_applied;
            self.windows[target].zoomed = old_target_zoomed;
            self.windows[target].invalidate_frame();
            self.windows[source].panes.insert(source_pane, moved);
            self.windows[source].layout = old_source_layout;
            let _ = self.windows[source].apply_geometry(old_source_applied.geometry().clone());
            self.windows[source].applied = old_source_applied;
            self.windows[source].active = old_source_active;
            self.windows[source].zoomed = old_source_zoomed;
            self.windows[source].invalidate_frame();
            if self.windows[source].id == self.active_window && old_source_active == Some(pane) {
                let _ = self.windows[source].send_focus(Some(pane), true);
            }
            return Err(error);
        }

        if self.windows[source].is_empty() {
            self.close_window_index(source)?;
        } else if self.windows[source].id == self.active_window && old_source_active == Some(pane) {
            let active = self.windows[source].active;
            let _ = self.windows[source].send_focus(active, true);
        }
        self.chrome_generation = self.chrome_generation.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn send_all(
        &mut self,
        pane: Option<PaneId>,
        input: &[Vec<u8>],
        pace: Duration,
        mut tick: impl FnMut(&mut Self) -> Result<bool>,
    ) -> Result<()> {
        let target = match pane {
            Some(pane) => {
                self.pane_window_index(pane)
                    .ok_or_else(|| anyhow::anyhow!("workspace has no pane {pane}"))?;
                pane
            }
            None => self
                .selected_window()?
                .active_id()
                .context("workspace has no active pane")?,
        };
        for (index, bytes) in input.iter().enumerate() {
            self.send(Some(target), bytes)?;
            if index + 1 < input.len() {
                if pace.is_zero() {
                    if !tick(self)? {
                        bail!("workspace ended while sending input");
                    }
                } else {
                    let deadline = Instant::now() + pace;
                    while Instant::now() < deadline {
                        if !tick(self)? {
                            bail!("workspace ended while sending input");
                        }
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if !remaining.is_zero() {
                            std::thread::sleep(Duration::from_millis(10).min(remaining));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn capture(
        &mut self,
        pane: Option<PaneId>,
        settle: Duration,
        deadline: Duration,
        tick: impl FnMut(&mut Self) -> Result<bool>,
    ) -> Result<Shot> {
        let target_window = match pane {
            Some(pane) => self
                .pane_window_index(pane)
                .map(|index| self.windows[index].id)
                .ok_or_else(|| anyhow::anyhow!("workspace has no pane {pane}"))?,
            None => self.active_window,
        };
        self.capture_target(target_window, pane, settle, deadline, tick)
    }

    fn capture_target(
        &mut self,
        target_window: WindowId,
        pane: Option<PaneId>,
        settle: Duration,
        deadline: Duration,
        mut tick: impl FnMut(&mut Self) -> Result<bool>,
    ) -> Result<Shot> {
        let started = Instant::now();
        let deadline = started + deadline;
        loop {
            let running = tick(self)?;
            let Some(window_index) = self
                .windows
                .iter()
                .position(|window| window.id == target_window)
            else {
                bail!("window ended before capture completed");
            };
            if pane.is_none() && self.windows[window_index].all_exits_observed() {
                return self.shot_in_window(window_index, None);
            }
            if let Some(pane) = pane {
                let pane_index = self.windows[window_index].resolve_pane(Some(pane))?;
                if self.windows[window_index].panes[pane_index]
                    .session
                    .exit_observed()
                {
                    return self.windows[window_index].shot(Some(pane));
                }
            }
            if !running {
                return self.shot_in_window(window_index, pane);
            }
            let idle = match pane {
                Some(pane) => {
                    let pane_index = self.windows[window_index].resolve_pane(Some(pane))?;
                    self.windows[window_index].panes[pane_index]
                        .session
                        .idle_for(started)
                }
                None => self.windows[window_index]
                    .panes
                    .iter()
                    .map(|pane| pane.session.idle_for(started))
                    .min()
                    .unwrap_or_else(|| started.elapsed()),
            };
            if settle.is_zero() || idle >= settle || Instant::now() >= deadline {
                return self.shot_in_window(window_index, pane);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn shot_in_window(&mut self, window: usize, pane: Option<PaneId>) -> Result<Shot> {
        if let Some(pane) = pane {
            return self.windows[window].shot(Some(pane));
        }
        let selected = self.windows[window].id;
        let mut frame = self.windows[window].frame()?;
        self.add_tab_strip(&mut frame, selected);
        let ansi = frame_ansi(&frame)?;
        Ok(Shot { frame, ansi })
    }

    pub(crate) fn wait_for_text(
        &mut self,
        pane: Option<PaneId>,
        text: &str,
        timeout: Duration,
        mut tick: impl FnMut(&mut Self) -> Result<bool>,
    ) -> Result<()> {
        let target = match pane {
            Some(pane) => {
                self.pane_window_index(pane)
                    .ok_or_else(|| anyhow::anyhow!("workspace has no pane {pane}"))?;
                pane
            }
            None => self
                .selected_window()?
                .active_id()
                .context("workspace has no active pane")?,
        };
        let deadline = Instant::now() + timeout;
        loop {
            let running = tick(self)?;
            let Some(window_index) = self.pane_window_index(target) else {
                bail!("pane {target} ended before visible terminal included {text:?}");
            };
            let pane_index = self.windows[window_index].resolve_pane(Some(target))?;
            if self.windows[window_index].panes[pane_index]
                .session
                .current_frame()?
                .text()
                .contains(text)
            {
                return Ok(());
            }
            if !running
                || self.windows[window_index].panes[pane_index]
                    .session
                    .exit_observed()
            {
                bail!("pane ended before visible terminal included {text:?}");
            }
            if Instant::now() >= deadline {
                bail!("timed out waiting for visible terminal text {text:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub(crate) fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<()> {
        if self.windows.is_empty() {
            return Ok(());
        }
        let content_rows = content_rows(rows)?;
        let previous = (
            self.cols,
            self.rows,
            self.windows[0].options.cell_width,
            self.windows[0].options.cell_height,
        );
        if previous == (cols, rows, cell_width, cell_height) {
            return Ok(());
        }
        for index in 0..self.windows.len() {
            if let Err(error) =
                self.windows[index].resize(cols, content_rows, cell_width, cell_height)
            {
                for window in &mut self.windows[..=index] {
                    let _ = window.resize(
                        previous.0,
                        previous.1 - TAB_STRIP_ROWS,
                        previous.2,
                        previous.3,
                    );
                }
                return Err(error);
            }
        }
        self.cols = cols;
        self.rows = rows;
        self.chrome_generation = self.chrome_generation.wrapping_add(1);
        if let Some(recording) = &mut self.recording {
            recording
                .writer
                .resize(cols, rows, cell_width, cell_height)?;
            recording.window = None;
        }
        Ok(())
    }

    pub(crate) fn status(&mut self) -> Result<SessionStatus> {
        let active = self
            .active_window_index()
            .context("workspace has no active window")?;
        let mut statuses = Vec::with_capacity(self.windows.len());
        for window in &mut self.windows {
            statuses.push(window.status()?);
        }
        let mut status = aggregate_statuses(&statuses, active)?;
        status.cols = self.cols;
        status.rows = self.rows;
        status.launch = self.launch.clone();
        status.recording = self.recording.is_some();
        Ok(status)
    }

    pub(crate) fn take_bells(&mut self) -> u64 {
        let mut bells = 0;
        for window in &mut self.windows {
            let count = window.take_bells();
            if count > 0
                && window.id != self.active_window
                && window.mark_activity(ActivityKind::Bell)
            {
                self.chrome_generation = self.chrome_generation.wrapping_add(1);
            }
            bells += count;
        }
        bells
    }

    pub(crate) fn mark_recording(&mut self, name: &str) -> Result<()> {
        self.pump()?;
        if let Some(recording) = &mut self.recording {
            return recording.writer.marker(name);
        }
        bail!("workspace is not recording")
    }

    pub(crate) fn stop(&mut self) {
        let _ = self.try_stop();
    }

    pub(crate) fn try_stop(&mut self) -> Result<()> {
        for window in &mut self.windows {
            window.stop(&mut self.pending_input);
        }
        let result = self.flush_input(None);
        self.windows.clear();
        result
    }

    fn active_id(&self) -> Option<PaneId> {
        self.selected_window().ok().and_then(Window::active_id)
    }

    pub(crate) fn active_logs(&mut self, ansi: bool) -> Result<Vec<u8>> {
        self.selected_window_mut()?.active_logs(ansi)
    }

    #[cfg(test)]
    fn panes(&mut self) -> Result<Vec<PaneStatus>> {
        self.panes_in(None)
    }

    #[cfg(test)]
    fn set_grid(&mut self, columns: u16, rows: u16) -> Result<()> {
        self.set_grid_in(None, columns, rows).map(|_| ())
    }

    #[cfg(test)]
    fn shot(&mut self, pane: Option<PaneId>) -> Result<Shot> {
        self.selected_window_mut()?.shot(pane)
    }

    fn cancel_paste(&mut self) {
        if let Ok(window) = self.selected_window_mut() {
            window.cancel_paste();
        }
    }

    #[cfg(test)]
    fn frame(&mut self) -> Result<Frame> {
        let mut frame = self.selected_window_mut()?.frame()?;
        self.add_tab_strip(&mut frame, self.active_window);
        Ok(frame)
    }

    fn active_title(&self) -> Result<String> {
        self.selected_window()?.active_title()
    }

    fn active_input_modes(&self) -> Result<InputModes> {
        let mut modes = self.selected_window()?.active_input_modes()?;
        if self.windows.len() > 1 {
            modes.normal_mouse = true;
            modes.sgr_mouse = true;
        }
        Ok(modes)
    }

    fn send_active_if_open(&mut self, input: &[u8]) -> Result<bool> {
        self.selected_window_mut()?.send_active_if_open(input)
    }

    fn begin_paste(&mut self) -> Result<bool> {
        self.selected_window_mut()?.begin_paste()
    }

    fn send_paste(&mut self, input: &[u8]) -> Result<bool> {
        self.selected_window_mut()?.send_paste(input)
    }

    fn end_paste(&mut self) -> Result<bool> {
        self.selected_window_mut()?.end_paste()
    }

    fn focus_direction(&mut self, direction: Direction) -> Result<bool> {
        self.selected_window_mut()?.focus_direction(direction)
    }

    fn resize_active(&mut self, direction: Direction, cells: u16) -> Result<()> {
        let pane = self.active_id().context("workspace has no active pane")?;
        self.selected_window_mut()?
            .resize_pane(pane, direction, cells)
    }

    fn toggle_active_zoom(&mut self) -> Result<()> {
        let pane = self.active_id().context("workspace has no active pane")?;
        self.selected_window_mut()?.toggle_zoom(pane)
    }

    fn selected_pane_count(&self) -> usize {
        self.selected_window()
            .map_or(0, |window| window.panes.len())
    }

    fn selected_pane_ids(&self) -> Vec<PaneId> {
        self.selected_window()
            .map(|window| window.panes.iter().map(|pane| pane.id).collect())
            .unwrap_or_default()
    }

    fn is_multi_pane(&self) -> bool {
        self.selected_window().is_ok_and(Window::is_multi_pane)
    }

    fn pane_at(&self, x: u16, y: u16) -> Option<(PaneId, u16, u16)> {
        self.selected_window().ok()?.pane_at(x, self.content_y(y))
    }

    fn pane_position(&self, pane: PaneId, x: u16, y: u16) -> Option<(PaneId, u16, u16)> {
        self.selected_window()
            .ok()?
            .pane_position(pane, x, self.content_y(y))
    }

    fn pane_input_modes(&self, pane: PaneId) -> Result<InputModes> {
        self.selected_window()?.pane_input_modes(pane)
    }

    fn send_to_if_open(&mut self, pane: PaneId, input: &[u8]) -> Result<bool> {
        self.selected_window_mut()?.send_to_if_open(pane, input)
    }

    fn active_cursor_style(&self) -> Result<libghostty_vt::render::CursorVisualStyle> {
        Ok(self.selected_window()?.active_cursor_style())
    }

    fn active_presentation_revision(&self) -> Result<(WindowId, PaneId, u64, u64)> {
        let window = self.selected_window()?;
        let pane = window.active.context("workspace has no active pane")?;
        let index = window
            .pane_index(pane)
            .context("active pane is missing from its window")?;
        Ok((
            window.id,
            pane,
            window.panes[index].session.frame_revision(),
            self.chrome_generation,
        ))
    }

    fn active_frame_key(&self, revisions: &mut PaneRevisions) -> Result<(WindowId, u64, u64)> {
        let window = self.selected_window()?;
        window.pane_revisions(revisions);
        Ok((window.id, window.frame_generation, self.chrome_generation))
    }

    fn frame_with_revisions(&mut self, revisions: &PaneRevisions) -> Result<Frame> {
        let mut frame = self
            .selected_window_mut()?
            .frame_with_revisions(revisions)?;
        self.add_tab_strip(&mut frame, self.active_window);
        Ok(frame)
    }

    fn add_tab_strip(&self, frame: &mut Frame, selected: WindowId) {
        if self.tab_position == TabPosition::Top {
            for cell in &mut frame.cells {
                cell.y = cell.y.saturating_add(TAB_STRIP_ROWS);
            }
            if let Some(cursor) = &mut frame.cursor {
                cursor.y = cursor.y.saturating_add(TAB_STRIP_ROWS);
            }
        }
        frame.rows = self.rows;
        let y = self.tab_row();
        frame.cells.push(Cell {
            x: 0,
            y,
            text: String::new(),
            width: self.cols,
            foreground: frame.foreground,
            background: frame.background,
            attributes: Attributes::default(),
        });
        for tab in self.tab_labels(selected) {
            let window = &self.windows[tab.index];
            for (offset, character) in tab.text.chars().enumerate() {
                let Ok(offset) = u16::try_from(offset) else {
                    break;
                };
                let x = tab.start.saturating_add(offset);
                if x >= self.cols {
                    break;
                }
                let active = window.id == selected;
                frame.cells.push(Cell {
                    x,
                    y,
                    text: character.to_string(),
                    width: 1,
                    foreground: if active {
                        frame.background
                    } else {
                        frame.foreground
                    },
                    background: if active {
                        frame.foreground
                    } else {
                        frame.background
                    },
                    attributes: Attributes {
                        bold: active || !window.activity_kinds.is_empty(),
                        faint: !active && window.activity_kinds.is_empty(),
                        ..Attributes::default()
                    },
                });
            }
        }
    }

    fn tab_index_at(&self, x: u16, y: u16) -> Option<usize> {
        if y != self.tab_row() {
            return None;
        }
        self.tab_labels(self.active_window)
            .into_iter()
            .find_map(|tab| (x >= tab.start && x < tab.end).then_some(tab.index))
    }

    fn tab_drop_index_at(&self, x: u16, y: u16) -> Option<usize> {
        if y != self.tab_row() {
            return None;
        }
        let labels = self.tab_labels(self.active_window);
        labels
            .iter()
            .find(|tab| x < tab.end.saturating_add(1))
            .or_else(|| labels.last())
            .map(|tab| tab.index)
    }

    fn tab_labels(&self, selected: WindowId) -> Vec<TabLabel> {
        let mut x = 0_u16;
        self.windows
            .iter()
            .enumerate()
            .map_while(|(index, window)| {
                let badges = window
                    .activity_kinds
                    .iter()
                    .copied()
                    .map(ActivityKind::badge)
                    .collect::<String>();
                let activity = if badges.is_empty() {
                    String::new()
                } else {
                    format!(" {{{badges}}}")
                };
                let panes = if window.panes.len() > 1 {
                    format!(" {}p", window.panes.len())
                } else {
                    String::new()
                };
                let zoom = if window.zoomed.is_some() { " Z" } else { "" };
                let label = format!("{index}:{}{activity}{panes}{zoom}", window.name);
                let text = if window.id == selected {
                    format!("[{label}]")
                } else {
                    label
                };
                let width = u16::try_from(text.chars().count()).unwrap_or(u16::MAX);
                let start = x;
                let end = start.saturating_add(width).min(self.cols);
                x = end.saturating_add(2);
                (start < self.cols).then_some(TabLabel {
                    index,
                    start,
                    end,
                    text,
                })
            })
            .collect()
    }

    fn tab_row(&self) -> u16 {
        match self.tab_position {
            TabPosition::Top => 0,
            TabPosition::Bottom => self.rows.saturating_sub(TAB_STRIP_ROWS),
        }
    }

    fn content_y(&self, y: u16) -> u16 {
        match self.tab_position {
            TabPosition::Top => y.saturating_sub(TAB_STRIP_ROWS),
            TabPosition::Bottom => y,
        }
    }

    #[cfg(test)]
    fn set_selected_shell(&mut self, shell: Vec<String>) {
        if let Ok(window) = self.selected_window_mut() {
            window.shell = shell;
        }
    }
}

struct TabLabel {
    index: usize,
    start: u16,
    end: u16,
    text: String,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Window {
    fn start_with_theme(
        identity: WindowIdentity,
        pane_id: PaneId,
        command: &[String],
        cwd: Option<&Path>,
        options: &Options,
        theme: TerminalTheme,
        capture_input: bool,
    ) -> Result<Self> {
        let cwd = cwd
            .map(Path::to_owned)
            .unwrap_or(std::env::current_dir().context("resolve workspace directory")?);
        let shell = shell_command();
        let command = if command.is_empty() { &shell } else { command };
        let options = pane_options(options, &identity.workspace, identity.id, pane_id);
        let mut session = Session::start_with_theme(command, Some(&cwd), None, &options, theme)?;
        if capture_input {
            session.capture_input();
        }
        Ok(Self {
            id: identity.id,
            name: identity.name,
            workspace: identity.workspace,
            panes: vec![Pane {
                id: pane_id,
                session,
            }],
            active: Some(pane_id),
            layout: Some(LayoutNode::Leaf(pane_id)),
            applied: AppliedLayout::Ready(WorkspaceGeometry {
                panes: vec![PlacedPane {
                    id: pane_id,
                    rect: PaneRect {
                        x: 0,
                        y: 0,
                        cols: options.cols,
                        rows: options.rows,
                    },
                }],
                dividers: Vec::new(),
            }),
            cols: options.cols,
            rows: options.rows,
            cwd,
            shell,
            options: options.clone(),
            theme,
            paste: None,
            zoomed: None,
            cached_frame: None,
            frame_generation: 0,
            activity_kinds: BTreeSet::new(),
            capture_input,
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    fn all_exits_observed(&self) -> bool {
        !self.panes.is_empty() && self.panes.iter().all(|pane| pane.session.exit_observed())
    }

    pub(crate) fn active_id(&self) -> Option<PaneId> {
        self.active
    }

    fn is_multi_pane(&self) -> bool {
        self.panes.len() > 1
    }

    pub(crate) fn set_theme(&mut self, theme: TerminalTheme) -> Result<()> {
        if self.theme == theme {
            return Ok(());
        }
        let previous = self.theme;
        for index in 0..self.panes.len() {
            if let Err(error) = self.panes[index].session.set_theme(theme) {
                for pane in &mut self.panes[..=index] {
                    let _ = pane.session.set_theme(previous);
                }
                return Err(error);
            }
        }
        self.theme = theme;
        Ok(())
    }

    pub(crate) fn pump(&mut self, pending_input: &mut CapturedInput) -> Result<bool> {
        let mut changed = false;
        for pane in &mut self.panes {
            let before = pane.session.frame_revision();
            pane.session.pump()?;
            changed |= before != pane.session.frame_revision();
            pending_input.extend(pane.session.take_captured_input());
        }
        Ok(changed)
    }

    fn mark_activity(&mut self, kind: ActivityKind) -> bool {
        self.activity_kinds.insert(kind)
    }

    fn clear_activity(&mut self) {
        self.activity_kinds.clear();
    }

    pub(crate) fn observe_exits(&mut self) -> Result<bool> {
        let mut exited = false;
        for pane in &mut self.panes {
            exited |= pane.session.is_exited()?;
        }
        Ok(exited)
    }

    fn remove_observed_exits(
        &mut self,
        focused: bool,
        pending_input: &mut CapturedInput,
    ) -> Result<bool> {
        let exited = self
            .panes
            .iter()
            .filter(|pane| pane.session.exit_observed())
            .map(|pane| pane.id)
            .collect::<Vec<_>>();
        if exited.is_empty() {
            return Ok(false);
        }
        let active_removed = self.active.is_some_and(|active| exited.contains(&active));
        for id in &exited {
            self.remove_layout_pane(*id);
            let index = self
                .pane_index(*id)
                .context("observed exited pane is missing")?;
            let mut pane = self.panes.remove(index);
            pending_input.extend(pane.session.take_captured_input());
            if self.paste.is_some_and(|(target, _)| target == *id) {
                self.paste = None;
            }
        }
        if self.panes.is_empty() {
            self.active = None;
            self.layout = None;
            self.applied = AppliedLayout::Ready(WorkspaceGeometry {
                panes: Vec::new(),
                dividers: Vec::new(),
            });
            return Ok(true);
        }
        if active_removed {
            self.active = self.first_layout_pane();
            if focused {
                self.send_focus(self.active, true)?;
            }
        }
        self.refresh_layout()?;
        Ok(true)
    }

    fn split(
        &mut self,
        axis: SplitAxis,
        new_id: PaneId,
        pending_input: &mut CapturedInput,
    ) -> Result<()> {
        let active = self
            .active
            .ok_or_else(|| anyhow::anyhow!("workspace has no pane to split"))?;
        let mut layout = self
            .layout
            .clone()
            .ok_or_else(|| anyhow::anyhow!("workspace has no layout"))?;
        if !layout.split_leaf(active, axis, new_id) {
            bail!("workspace layout has no pane {active}");
        }
        let geometry = geometry(&layout, self.cols, self.rows)?;
        let rect = geometry
            .panes
            .iter()
            .find(|pane| pane.id == new_id)
            .map(|pane| pane.rect)
            .context("new pane has no layout rectangle")?;
        let pane = self.spawn_pane(new_id, rect, None)?;
        self.panes.push(pane);
        if let Err(error) = self.apply_geometry(geometry) {
            if let Some(pane) = self.panes.pop() {
                let _ = stop_pane(pane, pending_input);
            }
            return Err(error);
        }
        self.layout = Some(layout);
        self.focus_pane(new_id)
    }

    pub(crate) fn set_grid(
        &mut self,
        columns: u16,
        rows: u16,
        first_new_id: PaneId,
        command: Option<&[String]>,
        pending_input: &mut CapturedInput,
    ) -> Result<()> {
        if !(1..=2).contains(&columns) || !(1..=2).contains(&rows) {
            bail!("workspace grids support one or two columns and rows");
        }
        let desired = usize::from(columns) * usize::from(rows);
        if desired < self.panes.len() {
            bail!(
                "grid {columns}x{rows} has {desired} cells but workspace has {} panes; close panes explicitly first",
                self.panes.len()
            );
        }
        let mut ids = self.panes.iter().map(|pane| pane.id).collect::<Vec<_>>();
        ids.sort_unstable();
        while ids.len() < desired {
            ids.push(
                first_new_id
                    .checked_add(u32::try_from(ids.len() - self.panes.len()).unwrap_or(0))
                    .context("workspace exhausted stable pane ids")?,
            );
        }
        let layout = grid_layout(&ids, columns, rows)?;
        if self.layout.as_ref() == Some(&layout) {
            return Ok(());
        }
        let geometry = geometry(&layout, self.cols, self.rows)?;
        let mut added = Vec::new();
        let mut command = command.filter(|command| !command.is_empty());
        for id in ids
            .iter()
            .copied()
            .filter(|id| self.pane_index(*id).is_none())
        {
            let rect = geometry
                .panes
                .iter()
                .find(|pane| pane.id == id)
                .map(|pane| pane.rect)
                .context("new pane has no layout rectangle")?;
            match self.spawn_pane(id, rect, command.take()) {
                Ok(pane) => added.push(pane),
                Err(error) => {
                    for pane in added {
                        let _ = stop_pane(pane, pending_input);
                    }
                    return Err(error);
                }
            }
        }
        let added_len = added.len();
        self.panes.extend(added);
        if let Err(error) = self.apply_geometry(geometry) {
            for _ in 0..added_len {
                if let Some(pane) = self.panes.pop() {
                    let _ = stop_pane(pane, pending_input);
                }
            }
            return Err(error);
        }
        self.layout = Some(layout);
        if self.zoomed.is_some() {
            self.refresh_layout()?;
        }
        Ok(())
    }

    fn focus_direction(&mut self, direction: Direction) -> Result<bool> {
        let active = match self.active {
            Some(active) => active,
            None => return Ok(false),
        };
        let panes = &self.applied.geometry().panes;
        let Some(current) = panes.iter().find(|pane| pane.id == active) else {
            return Ok(false);
        };
        let target = panes
            .iter()
            .filter(|pane| pane.id != active)
            .filter_map(|pane| directional_score(current.rect, pane.rect, pane.id, direction))
            .min_by_key(|score| *score)
            .map(|(_, _, _, pane)| pane);
        match target {
            Some(target) => {
                self.focus_pane(target)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub(crate) fn focus_pane(&mut self, pane: PaneId) -> Result<()> {
        self.resolve_pane(Some(pane))?;
        if self.zoomed.is_some_and(|zoomed| zoomed != pane) {
            self.zoomed = None;
            self.refresh_layout()?;
        }
        if self.active == Some(pane) {
            return Ok(());
        }
        let _ = self.send_focus(self.active, false);
        self.active = Some(pane);
        self.invalidate_frame();
        let _ = self.send_focus(self.active, true);
        Ok(())
    }

    fn resize_pane(&mut self, pane: PaneId, direction: Direction, cells: u16) -> Result<()> {
        self.resolve_pane(Some(pane))?;
        if self.zoomed.is_some() {
            bail!("unzoom the window before resizing panes");
        }
        let old_layout = self
            .layout
            .clone()
            .context("workspace has no layout to resize")?;
        let before = geometry(&old_layout, self.cols, self.rows)?;
        let before_rect = before
            .panes
            .iter()
            .find(|placed| placed.id == pane)
            .map(|placed| placed.rect)
            .context("pane has no layout rectangle")?;
        let amount = i16::try_from(cells).context("pane resize exceeds supported cell count")?;
        let mut layout = old_layout.clone();
        if !layout.resize_leaf(pane, direction, amount) {
            bail!("pane {pane} has no boundary to resize {}", direction.name());
        }
        let next = geometry(&layout, self.cols, self.rows)?;
        let next_rect = next
            .panes
            .iter()
            .find(|placed| placed.id == pane)
            .map(|placed| placed.rect)
            .context("pane has no resized layout rectangle")?;
        if before_rect == next_rect {
            bail!("pane {pane} cannot resize any farther {}", direction.name());
        }
        if let Err(error) = self.apply_geometry(next) {
            self.layout = Some(old_layout);
            return Err(error);
        }
        self.layout = Some(layout);
        Ok(())
    }

    fn toggle_zoom(&mut self, pane: PaneId) -> Result<()> {
        self.resolve_pane(Some(pane))?;
        if self.panes.len() < 2 {
            bail!("window needs at least two panes to zoom");
        }
        let previous = self.zoomed;
        self.zoomed = (self.zoomed != Some(pane)).then_some(pane);
        if let Err(error) = self.refresh_layout() {
            self.zoomed = previous;
            let _ = self.refresh_layout();
            return Err(error);
        }
        Ok(())
    }

    fn clear_zoom(&mut self) -> Result<()> {
        let previous = self.zoomed;
        if self.zoomed.take().is_some()
            && let Err(error) = self.refresh_layout()
        {
            self.zoomed = previous;
            let _ = self.refresh_layout();
            return Err(error);
        }
        Ok(())
    }

    fn pane_at(&self, x: u16, y: u16) -> Option<(PaneId, u16, u16)> {
        self.applied.geometry().panes.iter().find_map(|pane| {
            let local_x = x.checked_sub(pane.rect.x)?;
            let local_y = y.checked_sub(pane.rect.y)?;
            (local_x < pane.rect.cols && local_y < pane.rect.rows)
                .then_some((pane.id, local_x, local_y))
        })
    }

    fn pane_position(&self, pane: PaneId, x: u16, y: u16) -> Option<(PaneId, u16, u16)> {
        let rect = self
            .applied
            .geometry()
            .panes
            .iter()
            .find(|placed| placed.id == pane)?
            .rect;
        let local_x = x.saturating_sub(rect.x).min(rect.cols.checked_sub(1)?);
        let local_y = y.saturating_sub(rect.y).min(rect.rows.checked_sub(1)?);
        Some((pane, local_x, local_y))
    }

    fn close_pane(
        &mut self,
        pane: PaneId,
        focused: bool,
        pending_input: &mut CapturedInput,
    ) -> Result<()> {
        let index = self.resolve_pane(Some(pane))?;
        let closing_active = self.active == Some(pane);
        if closing_active && focused {
            self.send_focus(Some(pane), false)?;
        }
        self.remove_layout_pane(pane);
        if self.paste.is_some_and(|(target, _)| target == pane) {
            self.paste = None;
        }
        let pane = self.panes.remove(index);
        stop_pane(pane, pending_input)?;
        if self.panes.is_empty() {
            self.active = None;
            self.layout = None;
            self.applied = AppliedLayout::Ready(WorkspaceGeometry {
                panes: Vec::new(),
                dividers: Vec::new(),
            });
            return Ok(());
        }
        if closing_active {
            self.active = self.first_layout_pane();
        }
        self.refresh_layout()?;
        if closing_active && focused {
            self.send_focus(self.active, true)?;
        }
        Ok(())
    }

    pub(crate) fn send(&mut self, pane: Option<PaneId>, input: &[u8]) -> Result<()> {
        let index = self.resolve_pane(pane)?;
        self.panes[index].session.send_current(input)
    }

    pub(crate) fn send_active_if_open(&mut self, input: &[u8]) -> Result<bool> {
        let index = self.resolve_pane(None)?;
        self.panes[index].session.send_current_if_open(input)
    }

    fn send_to_if_open(&mut self, pane: PaneId, input: &[u8]) -> Result<bool> {
        let index = self.resolve_pane(Some(pane))?;
        self.panes[index].session.send_current_if_open(input)
    }

    pub(crate) fn begin_paste(&mut self) -> Result<bool> {
        let index = self.resolve_pane(None)?;
        let target = self.panes[index].id;
        let bracketed = self.panes[index].session.input_modes()?.bracketed_paste;
        if bracketed
            && !self.panes[index]
                .session
                .send_current_if_open(PASTE_START)?
        {
            return Ok(false);
        }
        self.paste = Some((target, bracketed));
        Ok(true)
    }

    pub(crate) fn send_paste(&mut self, input: &[u8]) -> Result<bool> {
        let Some((target, _)) = self.paste else {
            return Ok(true);
        };
        let index = self.resolve_pane(Some(target))?;
        self.panes[index].session.send_current_if_open(input)
    }

    pub(crate) fn end_paste(&mut self) -> Result<bool> {
        let Some((target, bracketed)) = self.paste.take() else {
            return Ok(true);
        };
        if bracketed {
            let index = self.resolve_pane(Some(target))?;
            return self.panes[index].session.send_current_if_open(PASTE_END);
        }
        Ok(true)
    }

    fn cancel_paste(&mut self) {
        let Some((target, bracketed)) = self.paste.take() else {
            return;
        };
        if bracketed && let Some(index) = self.pane_index(target) {
            let _ = self.panes[index].session.send_current_if_open(PASTE_END);
        }
    }

    pub(crate) fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<()> {
        if self.cols == cols
            && self.rows == rows
            && self.options.cell_width == cell_width
            && self.options.cell_height == cell_height
        {
            return Ok(());
        }
        self.cols = cols;
        self.rows = rows;
        self.options.cols = cols;
        self.options.rows = rows;
        self.options.cell_width = cell_width;
        self.options.cell_height = cell_height;
        self.refresh_layout()
    }

    pub(crate) fn frame(&mut self) -> Result<Frame> {
        if self.applied.is_constrained() {
            return self.constrained_frame();
        }
        let mut revisions = Vec::with_capacity(self.panes.len());
        self.pane_revisions(&mut revisions);
        self.frame_with_revisions(&revisions)
    }

    fn frame_with_revisions(&mut self, revisions: &PaneRevisions) -> Result<Frame> {
        if self.applied.is_constrained() {
            return self.constrained_frame();
        }
        if let Some((cached_revisions, frame)) = &self.cached_frame
            && cached_revisions == revisions
        {
            return Ok(frame.clone());
        }
        let mut frames = Vec::with_capacity(self.applied.geometry().panes.len());
        for placed in &self.applied.geometry().panes {
            let index = self
                .pane_index(placed.id)
                .ok_or_else(|| anyhow::anyhow!("layout references missing pane {}", placed.id))?;
            frames.push((placed.id, self.panes[index].session.current_frame()?));
        }
        let frame = compose_workspace(
            self.cols,
            self.rows,
            self.applied.geometry(),
            &frames,
            self.active,
        );
        self.cached_frame = Some((revisions.clone(), frame.clone()));
        Ok(frame)
    }

    fn pane_revisions(&self, revisions: &mut PaneRevisions) {
        revisions.clear();
        revisions.extend(
            self.panes
                .iter()
                .map(|pane| (pane.id, pane.session.frame_revision())),
        );
    }

    fn invalidate_frame(&mut self) {
        self.cached_frame = None;
        self.frame_generation = self.frame_generation.wrapping_add(1);
    }

    fn constrained_frame(&mut self) -> Result<Frame> {
        let index = self.resolve_pane(None)?;
        let mut frame = self.panes[index].session.current_frame()?;
        frame.cols = self.cols;
        frame.rows = self.rows;
        frame.cells.retain(|cell| {
            cell.x < frame.cols
                && cell.y < frame.rows
                && cell.x.saturating_add(cell.width) <= frame.cols
        });
        frame.cursor = frame
            .cursor
            .filter(|cursor| cursor.x < frame.cols && cursor.y < frame.rows);
        add_overlay(&mut frame, "layout too small");
        Ok(frame)
    }

    pub(crate) fn shot(&mut self, pane: Option<PaneId>) -> Result<Shot> {
        if let Some(pane) = pane {
            let index = self.resolve_pane(Some(pane))?;
            return self.panes[index].session.snapshot();
        }
        let frame = self.frame()?;
        let ansi = frame_ansi(&frame)?;
        Ok(Shot { frame, ansi })
    }

    pub(crate) fn panes(&mut self) -> Result<Vec<PaneStatus>> {
        let mut statuses = Vec::with_capacity(self.panes.len());
        let base = self
            .layout
            .as_ref()
            .and_then(|layout| geometry(layout, self.cols, self.rows).ok());
        for pane in &mut self.panes {
            let status = pane.session.status()?;
            let visible = self.zoomed.is_none_or(|zoomed| zoomed == pane.id);
            let rect = self
                .applied
                .geometry()
                .panes
                .iter()
                .find(|placed| placed.id == pane.id)
                .map(|placed| placed.rect)
                .or_else(|| {
                    base.as_ref()?
                        .panes
                        .iter()
                        .find_map(|placed| (placed.id == pane.id).then_some(placed.rect))
                })
                .context("pane has no applied layout rectangle")?;
            statuses.push(PaneStatus {
                id: pane.id,
                active: self.active == Some(pane.id),
                visible,
                state: status.state,
                x: rect.x,
                y: rect.y,
                cols: status.cols,
                rows: status.rows,
                title: pane.session.title()?,
                command: status.launch.command,
                cwd: status.launch.cwd,
            });
        }
        Ok(statuses)
    }

    pub(crate) fn status(&mut self) -> Result<SessionStatus> {
        let active = self.resolve_pane(None)?;
        let mut statuses = Vec::with_capacity(self.panes.len());
        for pane in &mut self.panes {
            statuses.push(pane.session.status()?);
        }
        let mut status = aggregate_statuses(&statuses, active)?;
        status.cols = self.cols;
        status.rows = self.rows;
        Ok(status)
    }

    pub(crate) fn active_input_modes(&self) -> Result<InputModes> {
        let index = self.resolve_pane(None)?;
        let modes = self.panes[index].session.input_modes()?;
        Ok(outer_input_modes(modes, self.panes.len()))
    }

    fn pane_input_modes(&self, pane: PaneId) -> Result<InputModes> {
        let index = self.resolve_pane(Some(pane))?;
        self.panes[index].session.input_modes()
    }

    pub(crate) fn active_title(&self) -> Result<String> {
        let index = self.resolve_pane(None)?;
        self.panes[index].session.title()
    }

    pub(crate) fn take_bells(&self) -> u64 {
        self.panes
            .iter()
            .map(|pane| pane.session.take_bells())
            .sum()
    }

    pub(crate) fn active_cursor_style(&self) -> libghostty_vt::render::CursorVisualStyle {
        self.active
            .and_then(|active| self.pane_index(active))
            .map_or(libghostty_vt::render::CursorVisualStyle::Block, |index| {
                self.panes[index].session.cursor_style()
            })
    }

    pub(crate) fn active_logs(&mut self, ansi: bool) -> Result<Vec<u8>> {
        let index = self.resolve_pane(None)?;
        self.panes[index].session.logs(ansi)
    }

    pub(crate) fn stop(&mut self, pending_input: &mut CapturedInput) {
        self.paste = None;
        self.zoomed = None;
        self.invalidate_frame();
        for pane in self.panes.drain(..) {
            let _ = stop_pane(pane, pending_input);
        }
        self.active = None;
        self.layout = None;
        self.applied = AppliedLayout::Ready(WorkspaceGeometry {
            panes: Vec::new(),
            dividers: Vec::new(),
        });
    }

    fn refresh_layout(&mut self) -> Result<()> {
        let Some(layout) = &self.layout else {
            self.applied = AppliedLayout::Ready(WorkspaceGeometry {
                panes: Vec::new(),
                dividers: Vec::new(),
            });
            self.invalidate_frame();
            return Ok(());
        };
        let display = self
            .zoomed
            .map(LayoutNode::Leaf)
            .unwrap_or_else(|| layout.clone());
        match geometry(&display, self.cols, self.rows) {
            Ok(geometry) => self.apply_geometry(geometry),
            Err(_) => {
                self.applied = AppliedLayout::Constrained(self.applied.geometry().clone());
                self.invalidate_frame();
                Ok(())
            }
        }
    }

    fn apply_geometry(&mut self, geometry: WorkspaceGeometry) -> Result<()> {
        let previous = self.applied.geometry().clone();
        for placed in &geometry.panes {
            let index = self
                .pane_index(placed.id)
                .ok_or_else(|| anyhow::anyhow!("layout references missing pane {}", placed.id))?;
            if let Err(error) = self.panes[index].session.resize(
                placed.rect.cols,
                placed.rect.rows,
                self.options.cell_width,
                self.options.cell_height,
            ) {
                for previous in &previous.panes {
                    if let Some(index) = self.pane_index(previous.id) {
                        let _ = self.panes[index].session.resize(
                            previous.rect.cols,
                            previous.rect.rows,
                            self.options.cell_width,
                            self.options.cell_height,
                        );
                    }
                }
                return Err(error);
            }
        }
        self.applied = AppliedLayout::Ready(geometry);
        self.invalidate_frame();
        Ok(())
    }

    fn spawn_pane(&self, id: PaneId, rect: PaneRect, command: Option<&[String]>) -> Result<Pane> {
        let mut options = self.options.clone();
        options.cols = rect.cols;
        options.rows = rect.rows;
        let options = pane_options(&options, &self.workspace, self.id, id);
        let mut session = Session::start_with_theme(
            command.unwrap_or(&self.shell),
            Some(&self.cwd),
            None,
            &options,
            self.theme,
        )?;
        if self.capture_input {
            session.capture_input();
        }
        Ok(Pane { id, session })
    }

    fn remove_layout_pane(&mut self, pane: PaneId) {
        self.invalidate_frame();
        if self.zoomed == Some(pane) {
            self.zoomed = None;
        }
        if let Some(layout) = self.layout.take() {
            let (layout, removed) = layout.remove_leaf(pane);
            debug_assert!(removed, "pane collection and layout tree diverged");
            self.layout = layout;
        }
        self.applied
            .geometry_mut()
            .panes
            .retain(|placed| placed.id != pane);
    }

    fn first_layout_pane(&self) -> Option<PaneId> {
        self.layout.as_ref().map(LayoutNode::first_leaf)
    }

    fn pane_index(&self, pane: PaneId) -> Option<usize> {
        self.panes.iter().position(|candidate| candidate.id == pane)
    }

    fn send_focus(&mut self, pane: Option<PaneId>, focused: bool) -> Result<()> {
        let Some(index) = pane.and_then(|pane| self.pane_index(pane)) else {
            return Ok(());
        };
        if self.panes[index].session.input_modes()?.focus_events {
            self.panes[index].session.send_current_if_open(if focused {
                b"\x1b[I"
            } else {
                b"\x1b[O"
            })?;
        }
        Ok(())
    }

    fn resolve_pane(&self, pane: Option<PaneId>) -> Result<usize> {
        match pane {
            Some(id) => self
                .pane_index(id)
                .ok_or_else(|| anyhow::anyhow!("workspace has no pane {id}")),
            None => self
                .active
                .and_then(|active| self.pane_index(active))
                .ok_or_else(|| anyhow::anyhow!("workspace has no active pane")),
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.stop(&mut Vec::new());
    }
}

fn aggregate_statuses(statuses: &[SessionStatus], active: usize) -> Result<SessionStatus> {
    let mut status = statuses
        .get(active)
        .context("workspace has no active status")?
        .clone();
    status.idle_for_ms = statuses
        .iter()
        .filter_map(|status| status.idle_for_ms)
        .min();
    status.has_visible_content = statuses.iter().any(|status| status.has_visible_content);
    status.recording = statuses.iter().any(|status| status.recording);
    status.logs_truncated = statuses.iter().any(|status| status.logs_truncated);
    if statuses
        .iter()
        .any(|status| status.state == SessionState::Running)
    {
        status.state = SessionState::Running;
        status.exit = None;
    }
    Ok(status)
}

fn shell_command() -> Vec<String> {
    vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned())]
}

fn pane_options(options: &Options, workspace: &str, window: WindowId, pane: PaneId) -> Options {
    let mut options = options.clone();
    options
        .env
        .insert("TERMCTRL_SESSION".to_owned(), workspace.to_owned());
    options
        .env
        .insert("TERMCTRL_WORKSPACE".to_owned(), workspace.to_owned());
    options
        .env
        .insert("TERMCTRL_LAUNCH_WINDOW_ID".to_owned(), window.to_string());
    options
        .env
        .insert("TERMCTRL_PANE_ID".to_owned(), pane.to_string());
    options
}

fn content_rows(rows: u16) -> Result<u16> {
    rows.checked_sub(TAB_STRIP_ROWS)
        .filter(|rows| *rows > 0)
        .context("workspace needs at least two rows for content and tabs")
}

fn uncaptured_tab_position(
    position: Option<(u16, u16)>,
    rows: u16,
    tab_position: TabPosition,
    pane_captured: bool,
) -> Option<(u16, u16)> {
    let tab_row = match tab_position {
        TabPosition::Top => 0,
        TabPosition::Bottom => rows.saturating_sub(TAB_STRIP_ROWS),
    };
    position.filter(|(_, y)| !pane_captured && *y == tab_row)
}

fn validate_window_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("window name cannot be empty");
    }
    if name.len() > 64 {
        bail!("window name cannot exceed 64 bytes");
    }
    if !crate::session::valid_name(name) {
        bail!("window name may contain only ASCII letters, digits, '.', '-', and '_'");
    }
    Ok(())
}

fn geometry(layout: &LayoutNode, cols: u16, rows: u16) -> Result<WorkspaceGeometry> {
    let mut geometry = WorkspaceGeometry {
        panes: Vec::new(),
        dividers: Vec::new(),
    };
    place_layout(
        layout,
        PaneRect {
            x: 0,
            y: 0,
            cols,
            rows,
        },
        &mut geometry,
    )?;
    Ok(geometry)
}

fn grid_layout(panes: &[PaneId], columns: u16, rows: u16) -> Result<LayoutNode> {
    match (columns, rows, panes) {
        (1, 1, [pane]) => Ok(LayoutNode::Leaf(*pane)),
        (2, 1, [left, right]) => Ok(LayoutNode::Split {
            axis: SplitAxis::Columns,
            offset: 0,
            first: Box::new(LayoutNode::Leaf(*left)),
            second: Box::new(LayoutNode::Leaf(*right)),
        }),
        (1, 2, [top, bottom]) => Ok(LayoutNode::Split {
            axis: SplitAxis::Rows,
            offset: 0,
            first: Box::new(LayoutNode::Leaf(*top)),
            second: Box::new(LayoutNode::Leaf(*bottom)),
        }),
        (2, 2, [top_left, top_right, bottom_left, bottom_right]) => Ok(LayoutNode::Split {
            axis: SplitAxis::Rows,
            offset: 0,
            first: Box::new(LayoutNode::Split {
                axis: SplitAxis::Columns,
                offset: 0,
                first: Box::new(LayoutNode::Leaf(*top_left)),
                second: Box::new(LayoutNode::Leaf(*top_right)),
            }),
            second: Box::new(LayoutNode::Split {
                axis: SplitAxis::Columns,
                offset: 0,
                first: Box::new(LayoutNode::Leaf(*bottom_left)),
                second: Box::new(LayoutNode::Leaf(*bottom_right)),
            }),
        }),
        _ => bail!("workspace grids support 1x1, 2x1, 1x2, or 2x2"),
    }
}

fn place_layout(
    layout: &LayoutNode,
    rect: PaneRect,
    geometry: &mut WorkspaceGeometry,
) -> Result<()> {
    match layout {
        LayoutNode::Leaf(id) => geometry.panes.push(PlacedPane { id: *id, rect }),
        LayoutNode::Split {
            axis,
            offset,
            first,
            second,
        } => {
            let (first_rect, second_rect, divider) = match axis {
                SplitAxis::Columns if rect.cols >= 3 => {
                    let available = rect.cols - 1;
                    let first_cols = (i32::from(available / 2) + i32::from(*offset))
                        .clamp(1, i32::from(available - 1))
                        as u16;
                    (
                        PaneRect {
                            cols: first_cols,
                            ..rect
                        },
                        PaneRect {
                            x: rect.x + first_cols + 1,
                            cols: rect.cols - first_cols - 1,
                            ..rect
                        },
                        Divider {
                            axis: *axis,
                            x: rect.x + first_cols,
                            y: rect.y,
                            len: rect.rows,
                        },
                    )
                }
                SplitAxis::Rows if rect.rows >= 3 => {
                    let available = rect.rows - 1;
                    let first_rows = (i32::from(available / 2) + i32::from(*offset))
                        .clamp(1, i32::from(available - 1))
                        as u16;
                    (
                        PaneRect {
                            rows: first_rows,
                            ..rect
                        },
                        PaneRect {
                            y: rect.y + first_rows + 1,
                            rows: rect.rows - first_rows - 1,
                            ..rect
                        },
                        Divider {
                            axis: *axis,
                            x: rect.x,
                            y: rect.y + first_rows,
                            len: rect.cols,
                        },
                    )
                }
                SplitAxis::Columns => bail!("layout needs more columns"),
                SplitAxis::Rows => bail!("layout needs more rows"),
            };
            geometry.dividers.push(divider);
            place_layout(first, first_rect, geometry)?;
            place_layout(second, second_rect, geometry)?;
        }
    }
    Ok(())
}

fn compose_workspace(
    cols: u16,
    rows: u16,
    geometry: &WorkspaceGeometry,
    frames: &[(PaneId, Frame)],
    active: Option<PaneId>,
) -> Frame {
    let active_frame = active.and_then(|active| {
        frames
            .iter()
            .find(|(pane, _)| *pane == active)
            .map(|(_, frame)| frame)
    });
    let foreground = active_frame.map_or(DEFAULT_FOREGROUND, |frame| frame.foreground);
    let background = active_frame.map_or(DEFAULT_BACKGROUND, |frame| frame.background);
    let divider_cells = geometry
        .dividers
        .iter()
        .map(|divider| usize::from(divider.len))
        .sum::<usize>();
    let mut cells = Vec::with_capacity(
        frames
            .iter()
            .map(|(_, frame)| frame.cells.len())
            .sum::<usize>()
            + divider_cells,
    );
    for placed in &geometry.panes {
        let Some((_, frame)) = frames.iter().find(|(pane, _)| *pane == placed.id) else {
            continue;
        };
        if frame.background != background {
            for y in 0..placed.rect.rows {
                cells.push(Cell {
                    x: placed.rect.x,
                    y: placed.rect.y + y,
                    text: String::new(),
                    width: placed.rect.cols,
                    foreground: frame.foreground,
                    background: frame.background,
                    attributes: Attributes::default(),
                });
            }
        }
        for cell in &frame.cells {
            if cell.x >= placed.rect.cols
                || cell.y >= placed.rect.rows
                || cell.x.saturating_add(cell.width) > placed.rect.cols
            {
                continue;
            }
            let mut cell = cell.clone();
            cell.x += placed.rect.x;
            cell.y += placed.rect.y;
            cells.push(cell);
        }
    }
    let mut divider_cells = BTreeMap::new();
    for divider in &geometry.dividers {
        for offset in 0..divider.len {
            let x = divider.x
                + if divider.axis == SplitAxis::Rows {
                    offset
                } else {
                    0
                };
            let y = divider.y
                + if divider.axis == SplitAxis::Columns {
                    offset
                } else {
                    0
                };
            divider_cells
                .entry((x, y))
                .and_modify(|axes| *axes |= divider_axis(divider.axis))
                .or_insert_with(|| divider_axis(divider.axis));
        }
    }
    for (&(x, y), &axes) in &divider_cells {
        let left = x > 0 && divider_cells.contains_key(&(x - 1, y));
        let right = x + 1 < cols && divider_cells.contains_key(&(x + 1, y));
        let up = y > 0 && divider_cells.contains_key(&(x, y - 1));
        let down = y + 1 < rows && divider_cells.contains_key(&(x, y + 1));
        cells.push(Cell {
            x,
            y,
            text: divider_glyph(axes, left, right, up, down).to_owned(),
            width: 1,
            foreground,
            background,
            attributes: divider_attributes(),
        });
    }
    let cursor = active
        .and_then(|active| {
            geometry.panes.iter().find(|pane| pane.id == active).zip(
                frames
                    .iter()
                    .find(|(pane, _)| *pane == active)
                    .map(|(_, frame)| frame),
            )
        })
        .and_then(|(placed, frame)| {
            frame.cursor.as_ref().and_then(|cursor| {
                (cursor.x < placed.rect.cols && cursor.y < placed.rect.rows).then(|| Cursor {
                    x: placed.rect.x + cursor.x,
                    y: placed.rect.y + cursor.y,
                    color: cursor.color,
                    blinking: cursor.blinking,
                })
            })
        });
    Frame {
        version: FORMAT_VERSION,
        cols,
        rows,
        foreground,
        background,
        cursor,
        cells,
    }
}

fn divider_axis(axis: SplitAxis) -> u8 {
    match axis {
        SplitAxis::Columns => 1,
        SplitAxis::Rows => 2,
    }
}

fn divider_glyph(axes: u8, left: bool, right: bool, up: bool, down: bool) -> &'static str {
    match (left, right, up, down) {
        (true, true, true, true) => "┼",
        (true, true, false, true) => "┬",
        (true, true, true, false) => "┴",
        (false, true, true, true) => "├",
        (true, false, true, true) => "┤",
        (false, true, false, true) => "┌",
        (true, false, false, true) => "┐",
        (false, true, true, false) => "└",
        (true, false, true, false) => "┘",
        _ if axes & 1 != 0 => VERTICAL_DIVIDER,
        _ => HORIZONTAL_DIVIDER,
    }
}

fn divider_attributes() -> Attributes {
    Attributes {
        faint: true,
        ..Attributes::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum InputAction {
    Send(Vec<u8>),
    PasteStart,
    PasteData(Vec<u8>),
    PasteEnd,
    Split(SplitAxis),
    Focus(Direction),
    Resize(Direction),
    ToggleZoom,
    Mouse {
        input: Vec<u8>,
        position: Option<(u16, u16)>,
        primary_press: bool,
        capture_start: bool,
        captured_event: bool,
        capture_end: bool,
    },
    CloseActive,
    NewWindow,
    NextWindow,
    PreviousWindow,
    Palette,
    MoveWindow(isize),
    ToggleTabPosition,
    SelectWindow(usize),
    WindowList,
    CloseWindow,
    Detach,
    PaneNumbers,
    Quit,
    Help,
    Cancel,
    Unknown(u8),
}

struct PrefixDecoder {
    waiting: bool,
    pasting: bool,
    pending: Vec<u8>,
    pending_since: Option<Instant>,
    paste: Vec<u8>,
}

impl Default for PrefixDecoder {
    fn default() -> Self {
        Self {
            waiting: false,
            pasting: false,
            pending: Vec::with_capacity(PASTE_START.len()),
            pending_since: None,
            paste: Vec::new(),
        }
    }
}

impl PrefixDecoder {
    fn push(&mut self, bytes: &[u8]) -> Vec<InputAction> {
        let mut actions = Vec::new();
        let mut plain = Vec::new();
        let pending_since = self.pending_since.take();
        let mut input = std::mem::take(&mut self.pending);
        input.extend_from_slice(bytes);
        let mut index = 0;
        while index < input.len() {
            if self.pasting {
                let remaining = &input[index..];
                if let Some(end) = remaining
                    .windows(PASTE_END.len())
                    .position(|window| window == PASTE_END)
                {
                    self.paste.extend_from_slice(&remaining[..end]);
                    if !self.paste.is_empty() {
                        actions.push(InputAction::PasteData(std::mem::take(&mut self.paste)));
                    }
                    actions.push(InputAction::PasteEnd);
                    index += end + PASTE_END.len();
                    self.pasting = false;
                    continue;
                }
                let keep = partial_marker_len(remaining, PASTE_END);
                self.paste
                    .extend_from_slice(&remaining[..remaining.len() - keep]);
                if self.paste.len() >= PASTE_CHUNK_BYTES {
                    actions.push(InputAction::PasteData(std::mem::take(&mut self.paste)));
                }
                self.pending
                    .extend_from_slice(&remaining[remaining.len() - keep..]);
                self.pending_since = pending_since.or_else(|| Some(Instant::now()));
                break;
            }
            let remaining = &input[index..];
            if self.waiting {
                if let Some((direction, length)) = prefix_arrow(remaining) {
                    flush_plain(&mut actions, &mut plain);
                    actions.push(InputAction::Focus(direction));
                    self.waiting = false;
                    index += length;
                    continue;
                }
                if is_prefix_arrow_start(remaining) {
                    self.pending.extend_from_slice(remaining);
                    self.pending_since = pending_since.or_else(|| Some(Instant::now()));
                    break;
                }
            }
            if remaining.starts_with(SGR_MOUSE_PREFIX) {
                if let Some(end) = remaining
                    .iter()
                    .position(|byte| matches!(byte, b'M' | b'm'))
                {
                    flush_plain(&mut actions, &mut plain);
                    actions.push(sgr_mouse_action(&remaining[..=end]));
                    index += end + 1;
                    continue;
                }
                if remaining.len() > MAX_SGR_MOUSE_BYTES {
                    flush_plain(&mut actions, &mut plain);
                    if self.waiting {
                        actions.push(InputAction::Send(vec![PREFIX]));
                        self.waiting = false;
                    }
                    actions.push(InputAction::Send(remaining.to_vec()));
                    index = input.len();
                    continue;
                }
                self.pending.extend_from_slice(remaining);
                self.pending_since = pending_since.or_else(|| Some(Instant::now()));
                break;
            }
            if SGR_MOUSE_PREFIX.starts_with(remaining) {
                self.pending.extend_from_slice(remaining);
                self.pending_since = pending_since.or_else(|| Some(Instant::now()));
                break;
            }
            if remaining.starts_with(PASTE_START) {
                if self.waiting {
                    flush_plain(&mut actions, &mut plain);
                    actions.push(InputAction::Send(vec![PREFIX]));
                    self.waiting = false;
                }
                index += PASTE_START.len();
                self.pasting = true;
                actions.push(InputAction::PasteStart);
                continue;
            }
            if PASTE_START.starts_with(remaining) {
                self.pending.extend_from_slice(remaining);
                self.pending_since = pending_since.or_else(|| Some(Instant::now()));
                break;
            }
            let byte = input[index];
            index += 1;
            if self.waiting {
                flush_plain(&mut actions, &mut plain);
                let action = match byte {
                    b'%' => InputAction::Split(SplitAxis::Columns),
                    b'"' => InputAction::Split(SplitAxis::Rows),
                    b'h' => InputAction::Focus(Direction::Left),
                    b'k' => InputAction::Focus(Direction::Up),
                    b'j' => InputAction::Focus(Direction::Down),
                    b'H' => InputAction::Resize(Direction::Left),
                    b'L' => InputAction::Resize(Direction::Right),
                    b'K' => InputAction::Resize(Direction::Up),
                    b'J' => InputAction::Resize(Direction::Down),
                    b'z' => InputAction::ToggleZoom,
                    b'x' => InputAction::CloseActive,
                    b'c' => InputAction::NewWindow,
                    b'n' => InputAction::NextWindow,
                    b'l' => InputAction::PreviousWindow,
                    b'p' => InputAction::Palette,
                    b'<' => InputAction::MoveWindow(-1),
                    b'>' => InputAction::MoveWindow(1),
                    b't' => InputAction::ToggleTabPosition,
                    b'0'..=b'9' => InputAction::SelectWindow(usize::from(byte - b'0')),
                    b'w' => InputAction::WindowList,
                    b'd' => InputAction::Detach,
                    b'q' => InputAction::PaneNumbers,
                    b'&' => InputAction::CloseWindow,
                    b'Q' => InputAction::Quit,
                    b'?' => InputAction::Help,
                    0x1b => InputAction::Cancel,
                    PREFIX => InputAction::Send(vec![PREFIX]),
                    _ => InputAction::Unknown(byte),
                };
                actions.push(action);
                self.waiting = false;
            } else if byte == PREFIX {
                flush_plain(&mut actions, &mut plain);
                self.waiting = true;
            } else {
                plain.push(byte);
            }
        }
        flush_plain(&mut actions, &mut plain);
        actions
    }

    fn waiting(&self) -> bool {
        self.waiting
    }

    fn flush_ambiguous(&mut self, after: Duration) -> Vec<InputAction> {
        if self.pending.is_empty()
            || self
                .pending_since
                .is_none_or(|started| started.elapsed() < after)
        {
            return Vec::new();
        }
        self.pending_since = None;
        if self.pasting {
            return Vec::new();
        }
        let pending = std::mem::take(&mut self.pending);
        if pending.starts_with(SGR_MOUSE_PREFIX) {
            return Vec::new();
        }
        if self.waiting && pending.first() == Some(&0x1b) {
            self.waiting = false;
            let mut actions = vec![InputAction::Cancel];
            if pending.len() > 1 {
                actions.push(InputAction::Send(pending[1..].to_vec()));
            }
            return actions;
        }
        vec![InputAction::Send(pending)]
    }
}

fn prefix_arrow(bytes: &[u8]) -> Option<(Direction, usize)> {
    let direction = match bytes.get(..3)? {
        b"\x1b[D" | b"\x1bOD" => Direction::Left,
        b"\x1b[C" | b"\x1bOC" => Direction::Right,
        b"\x1b[A" | b"\x1bOA" => Direction::Up,
        b"\x1b[B" | b"\x1bOB" => Direction::Down,
        _ => return None,
    };
    Some((direction, 3))
}

fn is_prefix_arrow_start(bytes: &[u8]) -> bool {
    const ARROWS: [&[u8]; 8] = [
        b"\x1b[D", b"\x1bOD", b"\x1b[C", b"\x1bOC", b"\x1b[A", b"\x1bOA", b"\x1b[B", b"\x1bOB",
    ];
    ARROWS.iter().any(|arrow| arrow.starts_with(bytes))
}

fn sgr_mouse_action(bytes: &[u8]) -> InputAction {
    let parsed = (|| {
        let final_byte = *bytes.last()?;
        let body = std::str::from_utf8(&bytes[SGR_MOUSE_PREFIX.len()..bytes.len() - 1]).ok()?;
        let mut fields = body.split(';');
        let button = fields.next()?.parse::<u16>().ok()?;
        let x = fields.next()?.parse::<u16>().ok()?.checked_sub(1)?;
        let y = fields.next()?.parse::<u16>().ok()?.checked_sub(1)?;
        if fields.next().is_some() {
            return None;
        }
        let wheel = button & 0b1100_0000 != 0;
        let motion = button & 0b0010_0000 != 0;
        let press = final_byte == b'M' && !wheel && !motion && button & 0b11 != 3;
        let captured_event = (motion && button & 0b11 != 3) || final_byte == b'm';
        Some((
            (x, y),
            press && button & 0b11 == 0,
            press,
            captured_event,
            final_byte == b'm',
        ))
    })();
    InputAction::Mouse {
        input: bytes.to_vec(),
        position: parsed.map(|(position, _, _, _, _)| position),
        primary_press: parsed.is_some_and(|(_, primary_press, _, _, _)| primary_press),
        capture_start: parsed.is_some_and(|(_, _, capture_start, _, _)| capture_start),
        captured_event: parsed
            .is_some_and(|(_, _, _, captured_event, capture_end)| captured_event || capture_end),
        capture_end: parsed.is_some_and(|(_, _, _, _, capture_end)| capture_end),
    }
}

fn translate_mouse(input: &[u8], x: u16, y: u16, sgr: bool) -> Option<Vec<u8>> {
    if !input.starts_with(SGR_MOUSE_PREFIX) || input.len() <= SGR_MOUSE_PREFIX.len() + 1 {
        return None;
    }
    let final_byte = *input.last()?;
    let body = std::str::from_utf8(&input[SGR_MOUSE_PREFIX.len()..input.len() - 1]).ok()?;
    let mut button = body.split(';').next()?.parse::<u16>().ok()?;
    if sgr {
        return Some(
            format!(
                "\x1b[<{button};{};{}{}",
                x + 1,
                y + 1,
                char::from(final_byte)
            )
            .into_bytes(),
        );
    }
    if final_byte == b'm' {
        button = (button & !0b11) | 0b11;
    }
    Some(vec![
        0x1b,
        b'[',
        b'M',
        u8::try_from(button).ok()?.checked_add(32)?,
        u8::try_from(x).ok()?.checked_add(33)?,
        u8::try_from(y).ok()?.checked_add(33)?,
    ])
}

fn flush_plain(actions: &mut Vec<InputAction>, plain: &mut Vec<u8>) {
    if !plain.is_empty() {
        actions.push(InputAction::Send(std::mem::take(plain)));
    }
}

fn partial_marker_len(bytes: &[u8], marker: &[u8]) -> usize {
    (1..marker.len().min(bytes.len() + 1))
        .rev()
        .find(|&length| bytes.ends_with(&marker[..length]))
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArmedAction {
    Close(PaneId),
    CloseWindow(WindowId),
    Quit,
}

#[derive(Clone, Copy)]
enum PaletteCommand {
    SelectWindow(WindowId),
    FocusPane(PaneId),
    ToggleTabPosition,
    MoveWindow(isize),
    NewWindow,
    Detach,
}

struct PaletteItem {
    label: String,
    command: PaletteCommand,
}

#[derive(Default)]
struct CommandPalette {
    query: String,
    selected: usize,
}

struct WorkspaceUi {
    notice: Option<(String, Instant)>,
    armed: Option<(ArmedAction, Instant)>,
    palette: Option<CommandPalette>,
}

impl WorkspaceUi {
    fn new() -> Self {
        let mut ui = Self {
            notice: None,
            armed: None,
            palette: None,
        };
        ui.notice("^B ? workspace keys", Duration::from_secs(2));
        ui
    }

    fn notice(&mut self, text: impl Into<String>, duration: Duration) {
        self.notice = Some((text.into(), Instant::now() + duration));
    }

    fn clear_armed(&mut self) {
        if self.armed.take().is_some() {
            self.notice = None;
        }
    }

    fn open_palette(&mut self) {
        self.clear_armed();
        self.notice = None;
        self.palette = Some(CommandPalette::default());
    }

    fn palette_input(
        &mut self,
        workspace: &Workspace,
        input: &[u8],
    ) -> (Option<PaletteCommand>, Vec<u8>) {
        let mut index = 0;
        while index < input.len() {
            let remaining = &input[index..];
            let Some(palette) = self.palette.as_mut() else {
                return (None, input[index..].to_vec());
            };
            if remaining.starts_with(b"\x1b[A") || remaining.starts_with(b"\x1bOA") {
                palette.selected = palette.selected.saturating_sub(1);
                index += 3;
                continue;
            }
            if remaining.starts_with(b"\x1b[B") || remaining.starts_with(b"\x1bOB") {
                select_next_palette_item(workspace, palette);
                index += 3;
                continue;
            }
            match input[index] {
                0x1b | 0x03 => {
                    self.palette = None;
                    return (None, input[index + 1..].to_vec());
                }
                b'\r' | b'\n' => {
                    let command = palette_items(workspace, &palette.query)
                        .get(palette.selected)
                        .map(|item| item.command);
                    self.palette = None;
                    return (command, input[index + 1..].to_vec());
                }
                0x10 => palette.selected = palette.selected.saturating_sub(1),
                0x0e | b'\t' => select_next_palette_item(workspace, palette),
                0x7f | 0x08 => {
                    palette.query.pop();
                    palette.selected = 0;
                }
                byte if byte.is_ascii_graphic() || byte == b' ' => {
                    palette.query.push(char::from(byte));
                    palette.selected = 0;
                }
                _ => {}
            }
            index += 1;
        }
        (None, Vec::new())
    }

    fn arm(&mut self, action: ArmedAction, prompt: &str) {
        self.armed = Some((action, Instant::now() + Duration::from_secs(5)));
        self.notice(prompt, Duration::from_secs(5));
    }

    fn confirmation(&mut self, input: &[u8]) -> Option<Option<ArmedAction>> {
        let (action, expires) = self.armed?;
        if expires < Instant::now() {
            self.clear_armed();
            return None;
        }
        match input.first() {
            Some(b'y' | b'Y') => {
                self.armed = None;
                self.notice = None;
                Some(Some(action))
            }
            Some(b'n' | b'N' | 0x1b) => {
                self.clear_armed();
                Some(None)
            }
            _ => None,
        }
    }

    fn palette_lines(&self, workspace: &Workspace) -> Option<Vec<String>> {
        let palette = self.palette.as_ref()?;
        let items = palette_items(workspace, &palette.query);
        let mut lines = vec![format!("COMMAND PALETTE  > {}", palette.query)];
        if items.is_empty() {
            lines.push("  no matches".to_owned());
            return Some(lines);
        }
        let visible = 6_usize;
        let start = palette
            .selected
            .saturating_sub(visible / 2)
            .min(items.len().saturating_sub(visible));
        for (index, item) in items.iter().enumerate().skip(start).take(visible) {
            lines.push(format!(
                "{} {}",
                if index == palette.selected { '>' } else { ' ' },
                item.label
            ));
        }
        Some(lines)
    }

    fn overlay(&mut self, prefix: bool) -> Option<String> {
        if prefix {
            return Some("^B".to_owned());
        }
        let now = Instant::now();
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, expires)| *expires < now)
        {
            self.notice = None;
        }
        if self
            .armed
            .as_ref()
            .is_some_and(|(_, expires)| *expires < now)
        {
            self.armed = None;
        }
        self.notice.as_ref().map(|(text, _)| text.clone())
    }
}

fn select_next_palette_item(workspace: &Workspace, palette: &mut CommandPalette) {
    let length = palette_items(workspace, &palette.query).len();
    if length > 0 {
        palette.selected = (palette.selected + 1).min(length - 1);
    }
}

fn palette_items(workspace: &Workspace, query: &str) -> Vec<PaletteItem> {
    let mut items = Vec::new();
    for (index, window) in workspace.windows.iter().enumerate() {
        items.push(PaletteItem {
            label: format!("window {index}:{}", window.name),
            command: PaletteCommand::SelectWindow(window.id),
        });
        for pane in &window.panes {
            items.push(PaletteItem {
                label: format!("pane {} in {}", pane.id, window.name),
                command: PaletteCommand::FocusPane(pane.id),
            });
        }
    }
    items.extend([
        PaletteItem {
            label: "tabs toggle top/bottom".to_owned(),
            command: PaletteCommand::ToggleTabPosition,
        },
        PaletteItem {
            label: "window move left".to_owned(),
            command: PaletteCommand::MoveWindow(-1),
        },
        PaletteItem {
            label: "window move right".to_owned(),
            command: PaletteCommand::MoveWindow(1),
        },
        PaletteItem {
            label: "window new".to_owned(),
            command: PaletteCommand::NewWindow,
        },
        PaletteItem {
            label: "workspace detach".to_owned(),
            command: PaletteCommand::Detach,
        },
    ]);
    if query.is_empty() {
        return items;
    }
    let query = query.to_ascii_lowercase();
    items
        .into_iter()
        .filter(|item| fuzzy_matches(&item.label.to_ascii_lowercase(), &query))
        .collect()
}

fn fuzzy_matches(candidate: &str, query: &str) -> bool {
    let mut query = query.chars();
    let mut expected = query.next();
    for character in candidate.chars() {
        if expected == Some(character) {
            expected = query.next();
        }
    }
    expected.is_none()
}

fn apply_palette_command(
    workspace: &mut Workspace,
    ui: &mut WorkspaceUi,
    command: PaletteCommand,
) -> Result<bool> {
    match command {
        PaletteCommand::SelectWindow(window) => workspace.select_window_id(window)?,
        PaletteCommand::FocusPane(pane) => workspace.focus_pane(pane)?,
        PaletteCommand::ToggleTabPosition => {
            let position = match workspace.tab_position {
                TabPosition::Top => TabPosition::Bottom,
                TabPosition::Bottom => TabPosition::Top,
            };
            workspace.set_tab_position(position);
        }
        PaletteCommand::MoveWindow(offset) => {
            if !workspace.move_active_window(offset)? {
                bail!("window cannot move any farther");
            }
        }
        PaletteCommand::NewWindow => {
            workspace.create_window(None, &[], None)?;
        }
        PaletteCommand::Detach => return Ok(false),
    }
    ui.notice("palette action applied", Duration::from_millis(1_000));
    Ok(true)
}

pub(crate) struct WorkspaceTerminal {
    attachment: Option<WorkspaceAttachment>,
    decoder: PrefixDecoder,
    pending_actions: VecDeque<InputAction>,
    ui: WorkspaceUi,
    mouse_capture: Option<MouseCapture>,
    pending_removal: bool,
    finished: bool,
    presentation_revision: Option<(WindowId, PaneId, u64, u64)>,
    paint_window: Option<(WindowId, u64, u64)>,
    paint_revisions: PaneRevisions,
    revision_scratch: PaneRevisions,
    painted_overlay: Option<String>,
}

#[derive(Clone, Copy)]
enum MouseCapture {
    Pane(PaneId),
    Tab { window: WindowId, origin_x: u16 },
}

impl MouseCapture {
    fn dragged_tab(self, release_x: u16) -> Option<WindowId> {
        match self {
            Self::Tab { window, origin_x } if release_x != origin_x => Some(window),
            _ => None,
        }
    }
}

struct WorkspaceAttachment {
    id: u64,
    // This receiver must drop before `screen`; its writer joins the socket reader on drop.
    input: Receiver<Vec<u8>>,
    screen: OuterScreen,
}

pub(crate) struct WorkspaceAttachmentOptions {
    pub(crate) id: u64,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) cell_width: u16,
    pub(crate) cell_height: u16,
    pub(crate) theme: TerminalTheme,
}

impl WorkspaceTerminal {
    pub(crate) fn detached() -> Self {
        Self {
            attachment: None,
            decoder: PrefixDecoder::default(),
            pending_actions: VecDeque::with_capacity(8),
            ui: WorkspaceUi::new(),
            mouse_capture: None,
            pending_removal: false,
            finished: false,
            presentation_revision: None,
            paint_window: None,
            paint_revisions: Vec::new(),
            revision_scratch: Vec::new(),
            painted_overlay: None,
        }
    }

    pub(crate) fn is_attached(&self) -> bool {
        self.attachment.is_some()
    }

    pub(crate) fn attach(
        &mut self,
        workspace: &mut Workspace,
        input: Receiver<Vec<u8>>,
        writer: Box<dyn Write + Send>,
        options: WorkspaceAttachmentOptions,
    ) -> Result<()> {
        if self.is_attached() {
            bail!("workspace already has an attached terminal");
        }
        let screen = OuterScreen::enter(writer)?;
        let previous = workspace.selected_window().map(|window| {
            (
                window.theme,
                workspace.cols,
                workspace.rows,
                window.options.cell_width,
                window.options.cell_height,
            )
        })?;
        workspace.set_theme(options.theme)?;
        if let Err(error) = workspace.resize(
            options.cols,
            options.rows,
            options.cell_width,
            options.cell_height,
        ) {
            let _ = workspace.set_theme(previous.0);
            let _ = workspace.resize(previous.1, previous.2, previous.3, previous.4);
            return Err(error);
        }
        self.decoder = PrefixDecoder::default();
        self.pending_actions.clear();
        self.ui = WorkspaceUi::new();
        self.mouse_capture = None;
        self.presentation_revision = None;
        self.paint_window = None;
        self.paint_revisions.clear();
        self.revision_scratch.clear();
        self.painted_overlay = None;
        self.attachment = Some(WorkspaceAttachment {
            id: options.id,
            input,
            screen,
        });
        Ok(())
    }

    pub(crate) fn resize_attachment(
        &mut self,
        workspace: &mut Workspace,
        id: u64,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<()> {
        if self.attachment.as_ref().map(|attachment| attachment.id) != Some(id) {
            bail!("attachment is no longer active");
        }
        workspace.resize(cols, rows, cell_width, cell_height)?;
        Ok(())
    }

    pub(crate) fn finished(&self) -> bool {
        self.finished
    }

    pub(crate) fn tick(&mut self, workspace: &mut Workspace) -> Result<bool> {
        if self.finished {
            return Ok(false);
        }
        if self.pending_removal {
            workspace.remove_observed_exits()?;
            self.pending_removal = false;
            if workspace.is_empty() {
                self.finished = true;
                return Ok(false);
            }
        }
        workspace.pump()?;
        let exited = workspace.observe_exits()?;
        let bells = workspace.take_bells();
        let mut attachment = self.attachment.take();
        if let Some(attached) = attachment.as_mut() {
            let result = self.tick_attachment(workspace, attached, exited, bells);
            match result {
                Ok(true) => {}
                Ok(false) => {
                    workspace.cancel_paste();
                    attachment = None;
                }
                Err(error) if attachment_closed(&error) => {
                    workspace.cancel_paste();
                    attachment = None;
                }
                Err(error) => return Err(error),
            }
        }
        self.attachment = attachment;
        if workspace.is_empty() {
            self.finished = true;
            return Ok(false);
        }
        if exited {
            if workspace.all_exits_observed() {
                self.finished = true;
                return Ok(false);
            }
            self.pending_removal = true;
        }
        Ok(true)
    }

    fn tick_attachment(
        &mut self,
        workspace: &mut Workspace,
        attachment: &mut WorkspaceAttachment,
        exited: bool,
        bells: u64,
    ) -> Result<bool> {
        if bells > 0 {
            attachment.screen.bell()?;
        }
        if !exited {
            let mut input_bytes = 0;
            for _ in 0..MAX_ATTACHMENT_INPUTS_PER_TICK {
                if input_bytes >= MAX_ATTACHMENT_INPUT_BYTES_PER_TICK
                    || self.pending_actions.len() >= MAX_ATTACHMENT_ACTIONS_PER_TICK
                {
                    break;
                }
                match attachment.input.try_recv() {
                    Ok(input) => {
                        input_bytes = input_bytes.saturating_add(input.len());
                        self.pending_actions.extend(self.decoder.push(&input));
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(false),
                }
            }
            self.pending_actions
                .extend(self.decoder.flush_ambiguous(Duration::from_millis(25)));
            for _ in 0..MAX_ATTACHMENT_ACTIONS_PER_TICK {
                let Some(action) = self.pending_actions.pop_front() else {
                    break;
                };
                if self.ui.palette.is_some() {
                    let (command, remainder) = match &action {
                        InputAction::Send(input) | InputAction::PasteData(input) => {
                            self.ui.palette_input(workspace, input)
                        }
                        InputAction::Cancel => {
                            self.ui.palette = None;
                            (None, Vec::new())
                        }
                        _ => (None, Vec::new()),
                    };
                    if let Some(command) = command {
                        match apply_palette_command(workspace, &mut self.ui, command) {
                            Ok(true) => {}
                            Ok(false) => return Ok(false),
                            Err(error) => {
                                attachment.screen.bell()?;
                                self.ui
                                    .notice(error.to_string(), Duration::from_millis(1_500));
                            }
                        }
                    }
                    if !remainder.is_empty() {
                        self.pending_actions
                            .push_front(InputAction::Send(remainder));
                    }
                    continue;
                }
                match action {
                    InputAction::Send(input) => {
                        if let Some(confirmation) = self.ui.confirmation(&input) {
                            match confirmation {
                                Some(ArmedAction::Close(pane)) => {
                                    match workspace.close_pane(pane) {
                                        Ok(()) => self.ui.notice(
                                            format!("pane {pane} killed"),
                                            Duration::from_millis(1_200),
                                        ),
                                        Err(error) => {
                                            attachment.screen.bell()?;
                                            self.ui.notice(
                                                error.to_string(),
                                                Duration::from_millis(1_500),
                                            );
                                        }
                                    }
                                }
                                Some(ArmedAction::CloseWindow(window)) => {
                                    if let Some(index) = workspace
                                        .windows
                                        .iter()
                                        .position(|candidate| candidate.id == window)
                                    {
                                        workspace.close_window_index(index)?;
                                    }
                                }
                                Some(ArmedAction::Quit) => workspace.try_stop()?,
                                None => self.ui.notice("canceled", Duration::from_millis(1_000)),
                            }
                            if workspace.is_empty() {
                                return Ok(true);
                            }
                            continue;
                        }
                        self.ui.clear_armed();
                        if !workspace.send_active_if_open(&input)? {
                            workspace.observe_exits()?;
                            break;
                        }
                    }
                    InputAction::PasteStart => {
                        self.ui.clear_armed();
                        if !workspace.begin_paste()? {
                            workspace.observe_exits()?;
                            break;
                        }
                    }
                    InputAction::PasteData(input) => {
                        if !workspace.send_paste(&input)? {
                            workspace.observe_exits()?;
                            break;
                        }
                    }
                    InputAction::PasteEnd => {
                        if !workspace.end_paste()? {
                            workspace.observe_exits()?;
                            break;
                        }
                    }
                    InputAction::Split(split) => {
                        self.ui.clear_armed();
                        match workspace.split(split) {
                            Ok(()) => self.ui.notice(
                                format!("pane {} active", workspace.active_id().unwrap_or(0)),
                                Duration::from_millis(1_200),
                            ),
                            Err(error) => {
                                attachment.screen.bell()?;
                                self.ui
                                    .notice(error.to_string(), Duration::from_millis(1_500));
                            }
                        }
                    }
                    InputAction::Focus(direction) => {
                        self.ui.clear_armed();
                        if workspace.focus_direction(direction)? {
                            self.ui.notice(
                                format!("pane {} active", workspace.active_id().unwrap_or(0)),
                                Duration::from_millis(1_000),
                            );
                        } else {
                            self.ui
                                .notice(direction.unavailable(), Duration::from_millis(1_000));
                        }
                    }
                    InputAction::Resize(direction) => {
                        self.ui.clear_armed();
                        match workspace.resize_active(direction, 5) {
                            Ok(()) => self.ui.notice(
                                format!("pane resized {}", direction.name()),
                                Duration::from_millis(1_000),
                            ),
                            Err(error) => {
                                attachment.screen.bell()?;
                                self.ui
                                    .notice(error.to_string(), Duration::from_millis(1_500));
                            }
                        }
                    }
                    InputAction::ToggleZoom => {
                        self.ui.clear_armed();
                        match workspace.toggle_active_zoom() {
                            Ok(()) => self
                                .ui
                                .notice("pane zoom toggled", Duration::from_millis(1_000)),
                            Err(error) => {
                                attachment.screen.bell()?;
                                self.ui
                                    .notice(error.to_string(), Duration::from_millis(1_500));
                            }
                        }
                    }
                    InputAction::Mouse {
                        input,
                        position,
                        primary_press,
                        capture_start,
                        captured_event,
                        capture_end,
                    } => {
                        if let Some((x, y)) = uncaptured_tab_position(
                            position,
                            workspace.rows,
                            workspace.tab_position,
                            matches!(self.mouse_capture, Some(MouseCapture::Pane(_))),
                        ) {
                            let target = workspace.tab_index_at(x, y);
                            if primary_press && let Some(index) = target {
                                self.ui.clear_armed();
                                self.mouse_capture =
                                    workspace
                                        .windows
                                        .get(index)
                                        .map(|window| MouseCapture::Tab {
                                            window: window.id,
                                            origin_x: x,
                                        });
                                workspace.select_window_index(index)?;
                            }
                            if capture_end {
                                let source = self
                                    .mouse_capture
                                    .take()
                                    .and_then(|capture| capture.dragged_tab(x));
                                let drop_target = workspace.tab_drop_index_at(x, y);
                                if let (Some(window), Some(target)) = (source, drop_target)
                                    && let Some(index) = workspace
                                        .windows
                                        .iter()
                                        .position(|candidate| candidate.id == window)
                                {
                                    let name = workspace.windows[index].name.clone();
                                    if let Err(error) = workspace.move_window(&name, target) {
                                        attachment.screen.bell()?;
                                        self.ui.notice(
                                            error.to_string(),
                                            Duration::from_millis(1_500),
                                        );
                                    }
                                }
                            }
                            continue;
                        }
                        if matches!(self.mouse_capture, Some(MouseCapture::Tab { .. })) {
                            if capture_end {
                                self.mouse_capture = None;
                            }
                            continue;
                        }
                        if workspace.is_multi_pane()
                            || matches!(self.mouse_capture, Some(MouseCapture::Pane(_)))
                        {
                            let target = position.and_then(|(x, y)| {
                                if captured_event {
                                    match self.mouse_capture {
                                        Some(MouseCapture::Pane(pane)) => {
                                            workspace.pane_position(pane, x, y)
                                        }
                                        _ => None,
                                    }
                                } else {
                                    workspace.pane_at(x, y)
                                }
                            });
                            if capture_end {
                                self.mouse_capture = None;
                            }
                            if let Some((pane, local_x, local_y)) = target {
                                if capture_start {
                                    self.mouse_capture = Some(MouseCapture::Pane(pane));
                                }
                                if primary_press {
                                    self.ui.clear_armed();
                                    workspace.focus_pane(pane)?;
                                }
                                let modes = workspace.pane_input_modes(pane)?;
                                if (modes.normal_mouse || modes.button_mouse || modes.any_mouse)
                                    && let Some(input) =
                                        translate_mouse(&input, local_x, local_y, modes.sgr_mouse)
                                    && !workspace.send_to_if_open(pane, &input)?
                                {
                                    workspace.observe_exits()?;
                                    break;
                                }
                            }
                        } else if !workspace.send_active_if_open(&input)? {
                            workspace.observe_exits()?;
                            break;
                        }
                    }
                    InputAction::CloseActive => {
                        let pane = workspace.active_id().unwrap_or(0);
                        let prompt = if workspace.selected_pane_count() == 1 {
                            format!("kill final pane {pane} and end workspace? (y/n)")
                        } else {
                            format!("kill pane {pane}? (y/n)")
                        };
                        self.ui.arm(ArmedAction::Close(pane), &prompt);
                    }
                    InputAction::NewWindow => {
                        self.ui.clear_armed();
                        match workspace.create_window(None, &[], None) {
                            Ok(_) => self.ui.notice(
                                format!(
                                    "window {} active",
                                    workspace.active_window_name().unwrap_or("?")
                                ),
                                Duration::from_millis(1_200),
                            ),
                            Err(error) => {
                                attachment.screen.bell()?;
                                self.ui
                                    .notice(error.to_string(), Duration::from_millis(1_500));
                            }
                        }
                    }
                    InputAction::NextWindow => {
                        self.ui.clear_armed();
                        if workspace.select_relative_window(1)? {
                            self.ui.notice(
                                format!(
                                    "window {} active",
                                    workspace.active_window_name().unwrap_or("?")
                                ),
                                Duration::from_millis(1_000),
                            );
                        }
                    }
                    InputAction::PreviousWindow => {
                        self.ui.clear_armed();
                        if workspace.select_previous_window()? {
                            self.ui.notice(
                                format!(
                                    "window {} active",
                                    workspace.active_window_name().unwrap_or("?")
                                ),
                                Duration::from_millis(1_000),
                            );
                        }
                    }
                    InputAction::Palette => {
                        self.mouse_capture = None;
                        self.ui.open_palette();
                    }
                    InputAction::MoveWindow(offset) => {
                        self.ui.clear_armed();
                        if workspace.move_active_window(offset)? {
                            self.ui
                                .notice("window reordered", Duration::from_millis(1_000));
                        } else {
                            attachment.screen.bell()?;
                            self.ui.notice(
                                "window cannot move any farther",
                                Duration::from_millis(1_200),
                            );
                        }
                    }
                    InputAction::ToggleTabPosition => {
                        self.ui.clear_armed();
                        let position = match workspace.tab_position {
                            TabPosition::Top => TabPosition::Bottom,
                            TabPosition::Bottom => TabPosition::Top,
                        };
                        workspace.set_tab_position(position);
                        self.ui.notice(
                            format!("tabs moved to {}", position.as_str()),
                            Duration::from_millis(1_000),
                        );
                    }
                    InputAction::SelectWindow(index) => {
                        self.ui.clear_armed();
                        if workspace.select_window_index(index)? {
                            self.ui.notice(
                                format!(
                                    "window {} active",
                                    workspace.active_window_name().unwrap_or("?")
                                ),
                                Duration::from_millis(1_000),
                            );
                        } else {
                            attachment.screen.bell()?;
                            self.ui
                                .notice(format!("no window {index}"), Duration::from_millis(1_000));
                        }
                    }
                    InputAction::WindowList => {
                        self.ui.clear_armed();
                        let windows = workspace
                            .windows()
                            .into_iter()
                            .map(|window| {
                                if window.active {
                                    format!("[{}:{}]", window.index, window.name)
                                } else {
                                    format!("{}:{}", window.index, window.name)
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("  ");
                        self.ui.notice(windows, Duration::from_secs(4));
                    }
                    InputAction::CloseWindow => {
                        let window = workspace.active_window;
                        let name = workspace.active_window_name().unwrap_or("?");
                        self.ui.arm(
                            ArmedAction::CloseWindow(window),
                            &format!("kill window {name:?} and all its panes? (y/n)"),
                        );
                    }
                    InputAction::Detach => return Ok(false),
                    InputAction::PaneNumbers => {
                        self.ui.clear_armed();
                        let mut panes = workspace.selected_pane_ids();
                        panes.sort_unstable();
                        let active = workspace.active_id();
                        let panes = panes
                            .into_iter()
                            .map(|pane| {
                                if active == Some(pane) {
                                    format!("[{pane}]")
                                } else {
                                    pane.to_string()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("  ");
                        self.ui.notice(
                            format!("panes: {panes}  active pane in brackets"),
                            Duration::from_secs(4),
                        );
                    }
                    InputAction::Quit => {
                        self.ui
                            .arm(ArmedAction::Quit, "kill workspace and all panes? (y/n)");
                    }
                    InputAction::Help => {
                        self.ui.clear_armed();
                        self.ui.notice(
                            "^B p palette  l last  n next  </> move  t tabs  c new  0-9 select  %/\" split  H/J/K/L resize  z zoom  d detach",
                            Duration::from_secs(4),
                        );
                    }
                    InputAction::Cancel => {
                        self.ui.clear_armed();
                        self.ui
                            .notice("workspace prefix canceled", Duration::from_millis(1_000));
                    }
                    InputAction::Unknown(byte) => {
                        self.ui.clear_armed();
                        attachment.screen.bell()?;
                        let key = if byte.is_ascii_graphic() {
                            char::from(byte).to_string()
                        } else {
                            format!("0x{byte:02x}")
                        };
                        self.ui.notice(
                            format!("unknown workspace key: {key}  ^B ? for help"),
                            Duration::from_millis(1_500),
                        );
                    }
                }
                if workspace.is_empty() {
                    return Ok(true);
                }
            }
        }
        let palette = self.ui.palette_lines(workspace);
        let overlay = palette
            .as_ref()
            .map(|lines| lines.join("\n"))
            .or_else(|| self.ui.overlay(self.decoder.waiting()));
        let mut revisions = std::mem::take(&mut self.revision_scratch);
        let paint_window = workspace.active_frame_key(&mut revisions)?;
        if self.paint_window != Some(paint_window)
            || self.paint_revisions != revisions
            || self.painted_overlay != overlay
        {
            let mut frame = workspace.frame_with_revisions(&revisions)?;
            let presentation_revision = workspace.active_presentation_revision()?;
            if self.presentation_revision != Some(presentation_revision) {
                attachment.screen.sync_title(&workspace.active_title()?)?;
                attachment
                    .screen
                    .sync_input_modes(workspace.active_input_modes()?)?;
                attachment.screen.sync_cursor_style(
                    workspace.active_cursor_style()?,
                    frame.cursor.as_ref().is_some_and(|cursor| cursor.blinking),
                )?;
                self.presentation_revision = Some(presentation_revision);
            }
            if let Some(lines) = &palette {
                add_palette_overlay(&mut frame, lines, workspace.tab_position);
            } else if let Some(overlay) = &overlay {
                add_overlay_at(&mut frame, overlay, workspace.tab_row());
            }
            attachment.screen.paint(frame)?;
            self.paint_window = Some(paint_window);
            self.painted_overlay = overlay;
        } else {
            attachment.screen.flush()?;
        }
        self.paint_revisions.clear();
        self.paint_revisions.extend_from_slice(&revisions);
        self.revision_scratch = revisions;
        Ok(true)
    }
}

fn attachment_closed(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let Some(error) = cause.downcast_ref::<std::io::Error>() else {
            return false;
        };
        matches!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::NotConnected
                | std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::WouldBlock
                | std::io::ErrorKind::UnexpectedEof
        ) || error.raw_os_error() == Some(libc::EIO)
    })
}

fn add_overlay(frame: &mut Frame, text: &str) {
    let y = frame.rows.saturating_sub(1);
    add_overlay_at(frame, text, y);
}

fn add_palette_overlay(frame: &mut Frame, lines: &[String], tab_position: TabPosition) {
    if frame.cols == 0 || frame.rows <= TAB_STRIP_ROWS || lines.is_empty() {
        return;
    }
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .min(frame.rows - TAB_STRIP_ROWS);
    let width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .and_then(|width| u16::try_from(width.saturating_add(2)).ok())
        .unwrap_or(frame.cols)
        .min(frame.cols);
    let start_y = match tab_position {
        TabPosition::Top => TAB_STRIP_ROWS,
        TabPosition::Bottom => frame.rows - TAB_STRIP_ROWS - height,
    };
    for (offset, line) in lines.iter().take(usize::from(height)).enumerate() {
        let y = start_y + u16::try_from(offset).unwrap_or(0);
        frame.cells.retain(|cell| cell.y != y || cell.x >= width);
        frame.cells.push(Cell {
            x: 0,
            y,
            text: String::new(),
            width,
            foreground: frame.background,
            background: frame.foreground,
            attributes: Attributes::default(),
        });
        for (x, character) in format!(" {line}")
            .chars()
            .take(usize::from(width))
            .enumerate()
        {
            frame.cells.push(Cell {
                x: u16::try_from(x).unwrap_or(0),
                y,
                text: character.to_string(),
                width: 1,
                foreground: frame.background,
                background: frame.foreground,
                attributes: Attributes {
                    bold: offset == 0 || line.starts_with('>'),
                    ..Attributes::default()
                },
            });
        }
    }
    frame.cursor = None;
}

fn add_overlay_at(frame: &mut Frame, text: &str, y: u16) {
    if frame.cols == 0 || frame.rows == 0 || y >= frame.rows || text.is_empty() {
        return;
    }
    let text = text
        .chars()
        .rev()
        .take(usize::from(frame.cols))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    let width = u16::try_from(text.chars().count()).unwrap_or(frame.cols);
    let start = frame.cols - width;
    let mut replacements = Vec::new();
    for cell in &frame.cells {
        if cell.y == y && cell.x < start && cell.x.saturating_add(cell.width) > start {
            replacements.push(Cell {
                x: cell.x,
                y,
                text: String::new(),
                width: start - cell.x,
                foreground: cell.foreground,
                background: cell.background,
                attributes: cell.attributes.clone(),
            });
        }
    }
    frame.cells.retain(|cell| {
        cell.y != y || cell.x.saturating_add(cell.width) <= start || cell.x >= frame.cols
    });
    frame.cells.extend(replacements);
    for (offset, character) in text.chars().enumerate() {
        frame.cells.push(Cell {
            x: start + u16::try_from(offset).unwrap_or(0),
            y,
            text: character.to_string(),
            width: 1,
            foreground: frame.background,
            background: frame.foreground,
            attributes: Attributes {
                bold: true,
                ..Attributes::default()
            },
        });
    }
}

struct OuterScreen {
    writer: Box<dyn Write + Send>,
    modes: InputModes,
    previous: Option<Frame>,
    title: String,
    cursor_style: Option<(libghostty_vt::render::CursorVisualStyle, bool)>,
    output: Vec<u8>,
}

impl OuterScreen {
    pub(crate) fn enter(mut writer: Box<dyn Write + Send>) -> Result<Self> {
        writer
            .write_all(b"\x1b[22;0t\x1b[?1049h\x1b[?2004h\x1b[?25l\x1b[2J\x1b[H")
            .context("enter workspace screen")?;
        writer.flush().context("flush workspace screen")?;
        Ok(Self {
            writer,
            modes: InputModes::default(),
            previous: None,
            title: String::new(),
            cursor_style: None,
            output: Vec::with_capacity(16 * 1024),
        })
    }

    pub(crate) fn sync_input_modes(&mut self, modes: InputModes) -> Result<()> {
        if modes == self.modes {
            return Ok(());
        }
        write_input_modes(&mut self.output, modes)?;
        self.modes = modes;
        Ok(())
    }

    pub(crate) fn paint(&mut self, frame: Frame) -> Result<()> {
        let changed = self.previous.as_ref() != Some(&frame);
        if changed {
            write_frame_update(&mut self.output, self.previous.as_ref(), &frame)?;
            self.previous = Some(frame);
        }
        self.flush()
    }

    fn flush(&mut self) -> Result<()> {
        if self.output.is_empty() {
            return Ok(());
        }
        self.writer
            .write_all(&self.output)
            .context("write workspace update")?;
        self.writer.flush().context("flush workspace frame")?;
        self.output.clear();
        Ok(())
    }

    pub(crate) fn sync_title(&mut self, title: &str) -> Result<()> {
        let title = title
            .chars()
            .filter(|character| !character.is_control())
            .collect::<String>();
        if title == self.title {
            return Ok(());
        }
        write!(self.output, "\x1b]2;{title}\x07").context("set workspace title")?;
        self.title = title;
        Ok(())
    }

    pub(crate) fn sync_cursor_style(
        &mut self,
        style: libghostty_vt::render::CursorVisualStyle,
        blinking: bool,
    ) -> Result<()> {
        if self.cursor_style == Some((style, blinking)) {
            return Ok(());
        }
        use libghostty_vt::render::CursorVisualStyle;
        let code = match (style, blinking) {
            (CursorVisualStyle::Block, true) => 1,
            (CursorVisualStyle::Block | CursorVisualStyle::BlockHollow, false) => 2,
            (CursorVisualStyle::Underline, true) => 3,
            (CursorVisualStyle::Underline, false) => 4,
            (CursorVisualStyle::Bar, true) => 5,
            (CursorVisualStyle::Bar, false) => 6,
            _ => 2,
        };
        write!(self.output, "\x1b[{code} q").context("set workspace cursor style")?;
        self.cursor_style = Some((style, blinking));
        Ok(())
    }

    pub(crate) fn bell(&mut self) -> Result<()> {
        self.output
            .write_all(b"\x07")
            .context("ring workspace bell")
    }
}

fn write_frame(mut writer: impl Write, frame: &Frame) -> Result<()> {
    write_frame_update(&mut writer, None, frame)
}

fn write_frame_update(
    mut writer: impl Write,
    previous: Option<&Frame>,
    frame: &Frame,
) -> Result<()> {
    let full = previous.is_none_or(|previous| {
        previous.cols != frame.cols
            || previous.rows != frame.rows
            || previous.foreground != frame.foreground
            || previous.background != frame.background
    });
    writer
        .write_all(b"\x1b[?2026h\x1b[?25l")
        .context("begin workspace frame")?;
    if full {
        write!(
            writer,
            "\x1b[0;48;2;{};{};{}m\x1b[2J\x1b[H",
            frame.background.r, frame.background.g, frame.background.b
        )
        .context("clear workspace screen")?;
    }
    let rows = cells_by_row(frame);
    let previous_rows = previous.map(cells_by_row);
    for y in 0..frame.rows {
        let cells = &rows[usize::from(y)];
        if !full
            && previous_rows
                .as_ref()
                .is_some_and(|previous| previous[usize::from(y)].as_slice() == cells.as_slice())
        {
            continue;
        }
        let row = dense_row(frame, cells);
        write!(writer, "\x1b[{};1H", y + 1).context("place workspace row")?;
        let mut style = None;
        let mut x = 0_usize;
        while x < row.len() {
            let cell = &row[x];
            if cell.continuation {
                x += 1;
                continue;
            }
            let next_style = (&cell.foreground, &cell.background, &cell.attributes);
            if style != Some(next_style) {
                write!(
                    writer,
                    "\x1b[0;38;2;{};{};{};48;2;{};{};{}{}m",
                    cell.foreground.r,
                    cell.foreground.g,
                    cell.foreground.b,
                    cell.background.r,
                    cell.background.g,
                    cell.background.b,
                    attributes(&cell.attributes),
                )
                .context("paint workspace style")?;
                style = Some(next_style);
            }
            writer
                .write_all(cell.text.unwrap_or(" ").as_bytes())
                .context("paint workspace text")?;
            x += usize::from(cell.width.max(1));
        }
    }
    if let Some(cursor) = &frame.cursor {
        write!(
            writer,
            "\x1b[0m\x1b[{};{}H\x1b[?25h",
            cursor.y + 1,
            cursor.x + 1
        )
        .context("place workspace cursor")?;
    }
    writer
        .write_all(b"\x1b[?2026l")
        .context("finish workspace frame")?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PaintCell<'a> {
    text: Option<&'a str>,
    width: u16,
    foreground: crate::frame::Color,
    background: crate::frame::Color,
    attributes: Attributes,
    continuation: bool,
}

fn cells_by_row(frame: &Frame) -> Vec<Vec<&Cell>> {
    let mut rows = vec![Vec::new(); usize::from(frame.rows)];
    for cell in &frame.cells {
        if cell.y < frame.rows {
            rows[usize::from(cell.y)].push(cell);
        }
    }
    rows
}

fn dense_row<'a>(frame: &Frame, cells: &[&'a Cell]) -> Vec<PaintCell<'a>> {
    let mut row = vec![
        PaintCell {
            text: None,
            width: 1,
            foreground: frame.foreground,
            background: frame.background,
            attributes: Attributes::default(),
            continuation: false,
        };
        usize::from(frame.cols)
    ];
    for &cell in cells {
        if cell.x >= frame.cols || cell.width == 0 {
            continue;
        }
        let end = cell.x.saturating_add(cell.width).min(frame.cols);
        if cell.text.is_empty() {
            for x in cell.x..end {
                row[usize::from(x)] = PaintCell {
                    text: None,
                    width: 1,
                    foreground: cell.foreground,
                    background: cell.background,
                    attributes: cell.attributes.clone(),
                    continuation: false,
                };
            }
            continue;
        }
        row[usize::from(cell.x)] = PaintCell {
            text: Some(&cell.text),
            width: cell.width,
            foreground: cell.foreground,
            background: cell.background,
            attributes: cell.attributes.clone(),
            continuation: false,
        };
        for x in cell.x + 1..end {
            row[usize::from(x)].continuation = true;
        }
    }
    row
}

fn frame_ansi(frame: &Frame) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, frame)?;
    Ok(bytes)
}

impl Drop for OuterScreen {
    fn drop(&mut self) {
        let _ = self.writer.write_all(&self.output);
        let _ = self
            .writer
            .write_all(
                b"\x1b[?2026l\x1b[?1l\x1b>\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?2004l",
            );
        let _ = self
            .writer
            .write_all(b"\x1b[0 q\x1b[0m\x1b[?25h\x1b[?1049l\x1b[23;0t");
        let _ = self.writer.flush();
    }
}

fn set_dec_mode(writer: &mut impl Write, mode: u16, enabled: bool) -> Result<()> {
    write!(writer, "\x1b[?{mode}{}", if enabled { 'h' } else { 'l' })
        .context("set workspace terminal mode")
}

fn write_input_modes(writer: &mut impl Write, modes: InputModes) -> Result<()> {
    set_dec_mode(writer, 1, modes.cursor_keys)?;
    writer
        .write_all(if modes.keypad_keys {
            b"\x1b="
        } else {
            b"\x1b>"
        })
        .context("set workspace keypad mode")?;
    set_dec_mode(writer, 1000, modes.normal_mouse)?;
    set_dec_mode(writer, 1002, modes.button_mouse)?;
    set_dec_mode(writer, 1003, modes.any_mouse)?;
    set_dec_mode(writer, 1006, modes.sgr_mouse)?;
    set_dec_mode(writer, 1004, modes.focus_events)?;
    Ok(())
}

fn outer_input_modes(modes: InputModes, pane_count: usize) -> InputModes {
    if pane_count == 1 {
        modes
    } else {
        let mut modes = modes;
        modes.normal_mouse = true;
        modes.sgr_mouse = true;
        modes
    }
}

fn attributes(attributes: &Attributes) -> String {
    let mut output = String::new();
    if attributes.bold {
        output.push_str(";1");
    }
    if attributes.faint {
        output.push_str(";2");
    }
    if attributes.italic {
        output.push_str(";3");
    }
    if attributes.underline.is_some() {
        output.push_str(";4");
    }
    if attributes.strikethrough {
        output.push_str(";9");
    }
    if attributes.overline {
        output.push_str(";53");
    }
    if attributes.invisible {
        output.push_str(";8");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn frame(cols: u16, rows: u16, text: &str, cursor: Option<(u16, u16)>) -> Frame {
        Frame {
            version: FORMAT_VERSION,
            cols,
            rows,
            foreground: DEFAULT_FOREGROUND,
            background: DEFAULT_BACKGROUND,
            cursor: cursor.map(|(x, y)| Cursor {
                x,
                y,
                color: DEFAULT_FOREGROUND,
                blinking: false,
            }),
            cells: vec![Cell {
                x: 0,
                y: 0,
                text: text.to_owned(),
                width: 1,
                foreground: DEFAULT_FOREGROUND,
                background: DEFAULT_BACKGROUND,
                attributes: Attributes::default(),
            }],
        }
    }

    fn status(state: SessionState) -> SessionStatus {
        SessionStatus {
            state,
            exit: (state == SessionState::Exited).then_some(crate::session::ProcessExit {
                code: 0,
                signal: None,
                success: true,
            }),
            cols: 80,
            rows: 24,
            cell_width: 9,
            cell_height: 18,
            idle_for_ms: Some(10),
            has_visible_content: false,
            recording: false,
            logs_truncated: false,
            launch: SessionLaunch {
                command: vec!["sh".to_owned()],
                cwd: PathBuf::from("/tmp"),
                record: None,
                cols: 80,
                rows: 24,
                cell_width: 9,
                cell_height: 18,
                max_bytes: 1024,
                opentui_host: false,
                color: crate::shot::ColorMode::Auto,
            },
        }
    }

    #[test]
    fn aggregate_status_remains_running_while_any_pane_runs() {
        let aggregate = aggregate_statuses(
            &[status(SessionState::Exited), status(SessionState::Running)],
            0,
        )
        .unwrap();
        assert_eq!(aggregate.state, SessionState::Running);
        assert!(aggregate.exit.is_none());

        let aggregate = aggregate_statuses(
            &[status(SessionState::Exited), status(SessionState::Exited)],
            0,
        )
        .unwrap();
        assert_eq!(aggregate.state, SessionState::Exited);
        assert!(aggregate.exit.is_some());
    }

    #[test]
    fn workspace_domain_values_have_explicit_external_names() {
        assert_eq!(TabPosition::Top.as_str(), "top");
        assert_eq!(TabPosition::Bottom.as_str(), "bottom");
        assert_eq!(ActivityKind::Output.as_str(), "output");
        assert_eq!(ActivityKind::Bell.as_str(), "bell");
        assert_eq!(ActivityKind::Exit.as_str(), "exit");
    }

    #[test]
    fn single_pane_workspace_mirrors_child_mouse_modes() {
        let modes = InputModes {
            normal_mouse: true,
            button_mouse: true,
            any_mouse: true,
            sgr_mouse: true,
            ..InputModes::default()
        };
        let mut output = Vec::new();

        write_input_modes(&mut output, outer_input_modes(modes, 1)).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\x1b[?1000h"));
        assert!(output.contains("\x1b[?1002h"));
        assert!(output.contains("\x1b[?1003h"));
        assert!(output.contains("\x1b[?1006h"));
    }

    #[test]
    fn split_workspace_adds_click_focus_without_disabling_child_mouse_modes() {
        let modes = InputModes {
            normal_mouse: true,
            button_mouse: true,
            any_mouse: true,
            sgr_mouse: true,
            ..InputModes::default()
        };

        let outer = outer_input_modes(modes, 2);

        assert!(outer.normal_mouse);
        assert!(outer.button_mouse);
        assert!(outer.any_mouse);
        assert!(outer.sgr_mouse);
    }

    #[test]
    fn two_pane_layout_reserves_one_divider_column() {
        let layout = grid_layout(&[0, 1], 2, 1).unwrap();
        assert_eq!(
            geometry(&layout, 80, 24).unwrap().panes,
            [
                PlacedPane {
                    id: 0,
                    rect: PaneRect {
                        x: 0,
                        y: 0,
                        cols: 39,
                        rows: 24
                    },
                },
                PlacedPane {
                    id: 1,
                    rect: PaneRect {
                        x: 40,
                        y: 0,
                        cols: 40,
                        rows: 24
                    },
                }
            ]
        );
    }

    #[test]
    fn composition_offsets_right_cells_and_active_cursor() {
        let layout = grid_layout(&[0, 1], 2, 1).unwrap();
        let geometry = geometry(&layout, 11, 3).unwrap();
        let composed = compose_workspace(
            11,
            3,
            &geometry,
            &[
                (0, frame(5, 3, "L", Some((1, 1)))),
                (1, frame(5, 3, "R", Some((2, 2)))),
            ],
            Some(1),
        );

        assert!(
            composed
                .cells
                .iter()
                .any(|cell| cell.text == "R" && cell.x == 6)
        );
        assert_eq!(composed.cursor.as_ref().unwrap().x, 8);
        assert_eq!(composed.text(), "L    │R\n     │\n     │");
    }

    #[test]
    fn stacked_layout_reserves_one_divider_row_and_offsets_the_bottom_pane() {
        let layout = grid_layout(&[0, 1], 1, 2).unwrap();
        let geometry = geometry(&layout, 8, 5).unwrap();
        assert_eq!(
            geometry.panes,
            [
                PlacedPane {
                    id: 0,
                    rect: PaneRect {
                        x: 0,
                        y: 0,
                        cols: 8,
                        rows: 2,
                    },
                },
                PlacedPane {
                    id: 1,
                    rect: PaneRect {
                        x: 0,
                        y: 3,
                        cols: 8,
                        rows: 2,
                    },
                },
            ]
        );
        let composed = compose_workspace(
            8,
            5,
            &geometry,
            &[
                (0, frame(8, 2, "T", Some((1, 1)))),
                (1, frame(8, 2, "B", Some((2, 1)))),
            ],
            Some(1),
        );

        assert_eq!(composed.cursor.as_ref().unwrap().y, 4);
        assert_eq!(composed.text(), "T\n\n────────\nB");
    }

    #[test]
    fn stacked_layout_offsets_bottom_background_spans_and_rejects_short_screens() {
        assert_eq!(
            geometry(&grid_layout(&[0, 1], 1, 2).unwrap(), 8, 2)
                .unwrap_err()
                .to_string(),
            "layout needs more rows"
        );
        assert_eq!(
            geometry(&grid_layout(&[0, 1], 2, 1).unwrap(), 2, 8)
                .unwrap_err()
                .to_string(),
            "layout needs more columns"
        );

        let layout = grid_layout(&[0, 1], 1, 2).unwrap();
        let geometry = geometry(&layout, 8, 5).unwrap();
        let top = frame(8, 2, "T", None);
        let mut bottom = frame(8, 2, "B", None);
        bottom.background = crate::frame::Color { r: 1, g: 2, b: 3 };
        let composed = compose_workspace(8, 5, &geometry, &[(0, top), (1, bottom)], Some(0));

        assert!(composed.cells.iter().any(|cell| {
            cell.y == 3
                && cell.width == 8
                && cell.background == crate::frame::Color { r: 1, g: 2, b: 3 }
        }));
        assert!(!composed.cells.iter().any(|cell| {
            cell.y < 3 && cell.background == crate::frame::Color { r: 1, g: 2, b: 3 }
        }));
    }

    #[test]
    fn compositor_batches_full_rows_and_diffs_incremental_updates() {
        let mut first = frame(80, 24, "x", Some((0, 0)));
        first.cells = (0..24)
            .flat_map(|y| {
                (0..80).map(move |x| Cell {
                    x,
                    y,
                    text: "x".to_owned(),
                    width: 1,
                    foreground: DEFAULT_FOREGROUND,
                    background: DEFAULT_BACKGROUND,
                    attributes: Attributes::default(),
                })
            })
            .collect();
        let mut full = Vec::new();
        write_frame_update(&mut full, None, &first).unwrap();

        let mut second = first.clone();
        second.cells[0].text = "y".to_owned();
        second.cursor.as_mut().unwrap().x = 1;
        let mut incremental = Vec::new();
        write_frame_update(&mut incremental, Some(&first), &second).unwrap();

        assert!(full.len() < 5_000, "full frame was {} bytes", full.len());
        assert!(
            incremental.len() < 500,
            "incremental frame was {} bytes",
            incremental.len()
        );
        assert!(!incremental.windows(3).any(|bytes| bytes == b"[2J"));
    }

    #[test]
    fn batched_compositor_round_trips_wide_and_styled_cells() {
        let mut source = frame(8, 2, "界", Some((2, 0)));
        source.cells[0].width = 2;
        source.cells.push(Cell {
            x: 3,
            y: 1,
            text: "X".to_owned(),
            width: 1,
            foreground: crate::frame::Color { r: 1, g: 2, b: 3 },
            background: crate::frame::Color { r: 4, g: 5, b: 6 },
            attributes: Attributes {
                bold: true,
                underline: Some(crate::frame::Underline::Single),
                ..Attributes::default()
            },
        });

        let replayed = crate::shot::from_ansi(frame_ansi(&source).unwrap(), 2, 8, 100_000)
            .unwrap()
            .frame;

        assert_eq!(replayed.text(), source.text());
        let styled = replayed.cells.iter().find(|cell| cell.text == "X").unwrap();
        assert!(styled.attributes.bold);
        assert!(styled.attributes.underline.is_some());
        assert_eq!(styled.foreground, crate::frame::Color { r: 1, g: 2, b: 3 });
        assert_eq!(styled.background, crate::frame::Color { r: 4, g: 5, b: 6 });
    }

    #[test]
    fn prefix_decoder_keeps_state_across_input_chunks() {
        let mut decoder = PrefixDecoder::default();

        assert!(decoder.push(&[PREFIX]).is_empty());
        assert_eq!(decoder.push(b"%"), [InputAction::Split(SplitAxis::Columns)]);
        assert_eq!(
            decoder.push(&[PREFIX, b'"']),
            [InputAction::Split(SplitAxis::Rows)]
        );
        assert_eq!(
            decoder.push(&[PREFIX, b'j']),
            [InputAction::Focus(Direction::Down)]
        );
        assert_eq!(
            decoder.push(&[PREFIX, b'k']),
            [InputAction::Focus(Direction::Up)]
        );
        assert_eq!(decoder.push(&[PREFIX, b'd']), [InputAction::Detach]);
        assert_eq!(decoder.push(&[PREFIX, b'q']), [InputAction::PaneNumbers]);
        assert_eq!(decoder.push(&[PREFIX, b'c']), [InputAction::NewWindow]);
        assert_eq!(decoder.push(&[PREFIX, b'n']), [InputAction::NextWindow]);
        assert_eq!(decoder.push(&[PREFIX, b'p']), [InputAction::Palette]);
        assert_eq!(decoder.push(&[PREFIX, b'l']), [InputAction::PreviousWindow]);
        assert_eq!(decoder.push(&[PREFIX, b'<']), [InputAction::MoveWindow(-1)]);
        assert_eq!(decoder.push(&[PREFIX, b'>']), [InputAction::MoveWindow(1)]);
        assert_eq!(
            decoder.push(&[PREFIX, b't']),
            [InputAction::ToggleTabPosition]
        );
        assert_eq!(
            decoder.push(&[PREFIX, b'2']),
            [InputAction::SelectWindow(2)]
        );
        assert_eq!(decoder.push(&[PREFIX, b'w']), [InputAction::WindowList]);
        assert_eq!(decoder.push(&[PREFIX, b'&']), [InputAction::CloseWindow]);
        assert_eq!(decoder.push(&[PREFIX, b'Q']), [InputAction::Quit]);
        assert!(decoder.push(&[PREFIX, 0x1b]).is_empty());
        assert!(decoder.push(b"[").is_empty());
        assert_eq!(decoder.push(b"A"), [InputAction::Focus(Direction::Up)]);
        assert_eq!(
            decoder.push(&[PREFIX, 0x1b, b'O', b'D']),
            [InputAction::Focus(Direction::Left)]
        );
        assert_eq!(decoder.push(&[PREFIX, b'z']), [InputAction::ToggleZoom]);
        assert_eq!(
            decoder.push(&[PREFIX, PREFIX]),
            [InputAction::Send(vec![PREFIX])]
        );
    }

    #[test]
    fn prefix_decoder_bounds_unterminated_mouse_sequences() {
        let mut decoder = PrefixDecoder::default();
        let mut malformed = SGR_MOUSE_PREFIX.to_vec();
        malformed.extend(std::iter::repeat_n(b'1', MAX_SGR_MOUSE_BYTES));

        assert_eq!(
            decoder.push(&malformed),
            [InputAction::Send(malformed.clone())]
        );
        assert!(decoder.pending.is_empty());

        let mut prefixed = vec![PREFIX];
        prefixed.extend_from_slice(&malformed);
        assert_eq!(
            decoder.push(&prefixed),
            [
                InputAction::Send(vec![PREFIX]),
                InputAction::Send(malformed)
            ]
        );
        assert!(!decoder.waiting());
    }

    #[test]
    fn prefix_decoder_turns_chunked_left_clicks_into_workspace_focus() {
        let mut decoder = PrefixDecoder::default();

        assert!(decoder.push(b"\x1b[<0;42").is_empty());
        assert_eq!(
            decoder.push(b";13M"),
            [InputAction::Mouse {
                input: b"\x1b[<0;42;13M".to_vec(),
                position: Some((41, 12)),
                primary_press: true,
                capture_start: true,
                captured_event: false,
                capture_end: false,
            }]
        );
        assert_eq!(
            decoder.push(b"\x1b[<0;42;13m"),
            [InputAction::Mouse {
                input: b"\x1b[<0;42;13m".to_vec(),
                position: Some((41, 12)),
                primary_press: false,
                capture_start: false,
                captured_event: true,
                capture_end: true,
            }]
        );
        assert_eq!(
            decoder.push(b"\x1b[<64;42;13M"),
            [InputAction::Mouse {
                input: b"\x1b[<64;42;13M".to_vec(),
                position: Some((41, 12)),
                primary_press: false,
                capture_start: false,
                captured_event: false,
                capture_end: false,
            }]
        );
        assert_eq!(
            decoder.push(b"\x1b[<32;42;13M"),
            [InputAction::Mouse {
                input: b"\x1b[<32;42;13M".to_vec(),
                position: Some((41, 12)),
                primary_press: false,
                capture_start: false,
                captured_event: true,
                capture_end: false,
            }]
        );
        assert_eq!(
            translate_mouse(b"\x1b[<64;42;13M", 2, 3, true).as_deref(),
            Some(b"\x1b[<64;3;4M".as_slice())
        );
        assert_eq!(
            translate_mouse(b"\x1b[<0;42;13M", 2, 3, false).as_deref(),
            Some(b"\x1b[M #$".as_slice())
        );
    }

    #[test]
    fn tab_click_requires_horizontal_movement_before_it_becomes_a_drag() {
        let click = MouseCapture::Tab {
            window: 7,
            origin_x: 12,
        };
        let drag = MouseCapture::Tab {
            window: 7,
            origin_x: 12,
        };

        assert_eq!(click.dragged_tab(12), None);
        assert_eq!(drag.dragged_tab(13), Some(7));
    }

    #[test]
    fn prefix_decoder_never_interprets_commands_inside_bracketed_paste() {
        let mut decoder = PrefixDecoder::default();

        assert!(decoder.push(b"\x1b[20").is_empty());
        assert_eq!(decoder.push(b"0~A\x02%B\x1b[20"), [InputAction::PasteStart]);
        assert!(decoder.flush_ambiguous(Duration::ZERO).is_empty());
        assert_eq!(
            decoder.push(b"1~"),
            [
                InputAction::PasteData(b"A\x02%B".to_vec()),
                InputAction::PasteEnd
            ]
        );
        assert_eq!(
            decoder.push(b"\x02%"),
            [InputAction::Split(SplitAxis::Columns)]
        );
    }

    #[test]
    fn prefix_decoder_flushes_ambiguous_escape_input() {
        let mut decoder = PrefixDecoder::default();

        assert!(decoder.push(b"\x1b").is_empty());
        assert_eq!(
            decoder.flush_ambiguous(Duration::ZERO),
            [InputAction::Send(vec![0x1b])]
        );
        assert!(decoder.push(b"\x02\x1b").is_empty());
        assert_eq!(
            decoder.flush_ambiguous(Duration::ZERO),
            [InputAction::Cancel]
        );
    }

    #[test]
    fn large_paste_streams_data_inside_one_transaction() {
        let mut decoder = PrefixDecoder::default();
        let mut first = PASTE_START.to_vec();
        first.extend(std::iter::repeat_n(b'a', PASTE_CHUNK_BYTES));
        let mut actions = decoder.push(&first);
        let mut last = vec![b'b'; 70_000 - PASTE_CHUNK_BYTES];
        last.extend_from_slice(PASTE_END);
        actions.extend(decoder.push(&last));

        assert_eq!(
            actions
                .iter()
                .filter(|action| matches!(action, InputAction::PasteStart))
                .count(),
            1
        );
        assert_eq!(
            actions
                .iter()
                .filter(|action| matches!(action, InputAction::PasteEnd))
                .count(),
            1
        );
        assert_eq!(
            actions
                .iter()
                .map(|action| match action {
                    InputAction::PasteData(bytes) => bytes.len(),
                    _ => 0,
                })
                .sum::<usize>(),
            70_000
        );
    }

    #[test]
    fn destructive_confirmation_accepts_yes_and_rejects_no() {
        let mut ui = WorkspaceUi::new();

        ui.arm(ArmedAction::Quit, "confirm quit");
        assert_eq!(ui.overlay(false).as_deref(), Some("confirm quit"));
        assert_eq!(ui.confirmation(b"n\r"), Some(None));
        assert_eq!(ui.overlay(false), None);
        ui.arm(ArmedAction::Close(3), "confirm close");
        assert_eq!(ui.confirmation(b"y\r"), Some(Some(ArmedAction::Close(3))));
        assert_eq!(ui.overlay(false), None);
    }

    #[test]
    fn overlay_preserves_uncovered_background_span() {
        let mut source = frame(8, 1, "", None);
        source.cells = vec![Cell {
            x: 0,
            y: 0,
            text: String::new(),
            width: 8,
            foreground: DEFAULT_FOREGROUND,
            background: crate::frame::Color { r: 1, g: 2, b: 3 },
            attributes: Attributes::default(),
        }];

        add_overlay(&mut source, "OK");

        assert!(source.cells.iter().any(|cell| {
            cell.x == 0
                && cell.width == 6
                && cell.background == crate::frame::Color { r: 1, g: 2, b: 3 }
        }));
    }

    #[cfg(unix)]
    #[test]
    fn workspace_semantic_snapshot_is_empty_without_a_provider() {
        let mut workspace = Workspace::start(
            &["sh".to_owned(), "-c".to_owned(), "sleep 10".to_owned()],
            None,
            None,
            &Options::default(),
        )
        .unwrap();

        let snapshot = workspace
            .semantic_snapshot_in(None, None, Duration::from_secs(1))
            .unwrap();

        assert_eq!(snapshot["format"], "termctrl-semantic-snapshot-v1");
        assert_eq!(snapshot["nodes"], serde_json::json!([]));
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn real_workspace_splits_and_routes_agent_input() {
        let options = Options {
            cols: 21,
            rows: 4,
            ..Options::default()
        };
        let mut workspace = Workspace::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "printf LEFT; cat".to_owned(),
            ],
            None,
            None,
            &options,
        )
        .unwrap();
        workspace.set_selected_shell(vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "printf RIGHT; cat".to_owned(),
        ]);
        workspace.split(SplitAxis::Columns).unwrap();
        workspace.send(Some(1), b"AGENT\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let frame = loop {
            workspace.pump().unwrap();
            let frame = workspace.frame().unwrap();
            let text = frame.text();
            if text.contains("LEFT") && text.contains("RIGHT") && text.contains("AGENT") {
                break frame;
            }
            assert!(Instant::now() < deadline, "workspace output did not arrive");
            std::thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(frame.cols, 21);
        assert_eq!(workspace.panes().unwrap().len(), 2);
        assert!(!workspace.shot(None).unwrap().ansi.is_empty());
        workspace.resize(2, 4, 9, 18).unwrap();
        assert_eq!(workspace.status().unwrap().cols, 2);
        assert_eq!(workspace.frame().unwrap().cols, 2);
        workspace.resize(21, 4, 9, 18).unwrap();
        assert!(workspace.frame().unwrap().text().contains("LEFT"));
        workspace.send(Some(1), &[0x04]).unwrap();
        let error = workspace
            .wait_for_text(Some(1), "never", Duration::from_secs(2), |workspace| {
                workspace.pump()?;
                workspace.observe_exits()?;
                workspace.remove_observed_exits()?;
                Ok(!workspace.is_empty())
            })
            .unwrap_err();
        assert!(error.to_string().contains("pane 1 ended"));
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn workspace_recording_replays_composed_panes_windows_and_tabs() {
        let record = std::env::temp_dir().join(format!(
            "termctrl-workspace-recording-{}-{:?}.termctrl",
            std::process::id(),
            std::thread::current().id()
        ));
        let options = Options {
            cols: 31,
            rows: 7,
            ..Options::default()
        };
        let mut workspace = Workspace::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "printf '\\033[5nLEFT'; cat".to_owned(),
            ],
            None,
            Some(&record),
            &options,
        )
        .unwrap();
        workspace.set_selected_shell(vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "printf RIGHT; cat".to_owned(),
        ]);
        workspace.split(SplitAxis::Columns).unwrap();
        workspace
            .create_window(
                Some("other"),
                &[
                    "sh".to_owned(),
                    "-c".to_owned(),
                    "printf HIDDEN; cat".to_owned(),
                ],
                None,
            )
            .unwrap();
        workspace.select_window("main").unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            workspace.pump().unwrap();
            let text = workspace.frame().unwrap().text();
            if text.contains("LEFT") && text.contains("RIGHT") && text.contains("1:other {+}") {
                break;
            }
            assert!(Instant::now() < deadline, "workspace output did not arrive");
            std::thread::sleep(Duration::from_millis(10));
        }
        workspace.send(Some(0), b"CLIENT\n").unwrap();
        workspace.rename_window("other", "renamed").unwrap();
        workspace.mark_recording("composed").unwrap();
        assert!(workspace.status().unwrap().recording);
        workspace.send(Some(1), b"PANE-CLOSE\n").unwrap();
        workspace.close_pane(1).unwrap();
        workspace.send(Some(2), b"WINDOW-CLOSE\n").unwrap();
        workspace.close_window("renamed").unwrap();
        workspace
            .create_window(
                Some("short"),
                &[
                    "sh".to_owned(),
                    "-c".to_owned(),
                    "IFS= read -r line; printf '\\033[5n'".to_owned(),
                ],
                None,
            )
            .unwrap();
        workspace.send(Some(3), b"EXIT\n").unwrap();
        workspace.select_window("main").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while workspace.windows().len() > 1 {
            workspace.observe_exits().unwrap();
            workspace.remove_observed_exits().unwrap();
            assert!(Instant::now() < deadline, "short window did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
        workspace.send(Some(0), b"FINAL\n").unwrap();
        drop(workspace);

        let replayed = crate::recording::shot_at(&record, None, Some("composed")).unwrap();
        let text = replayed.frame.text();
        assert!(text.contains("LEFT"), "{text:?}");
        assert!(text.contains("RIGHT"), "{text:?}");
        assert!(text.contains("[0:main 2p]"), "{text:?}");
        assert!(text.contains("1:renamed {+}"), "{text:?}");
        assert_eq!((replayed.frame.cols, replayed.frame.rows), (31, 7));
        let recording = crate::recording::read(&record).unwrap();
        assert!(recording.events.iter().any(|entry| matches!(
            entry,
            crate::recording::Entry::Input {
                origin: recording::InputOrigin::Client,
                bytes,
                ..
            } if bytes == b"CLIENT\n"
        )));
        for expected in [
            b"PANE-CLOSE\n".as_slice(),
            b"WINDOW-CLOSE\n".as_slice(),
            b"EXIT\n".as_slice(),
        ] {
            assert!(recording.events.iter().any(|entry| matches!(
                entry,
                crate::recording::Entry::Input {
                    origin: recording::InputOrigin::Client,
                    bytes,
                    ..
                } if bytes == expected
            )));
        }
        assert!(recording.events.iter().any(|entry| matches!(
            entry,
            crate::recording::Entry::Input {
                origin: recording::InputOrigin::Client,
                bytes,
                ..
            } if bytes == b"FINAL\n"
        )));
        assert!(recording.events.iter().any(|entry| matches!(
            entry,
            crate::recording::Entry::Input {
                origin: recording::InputOrigin::Host,
                ..
            }
        )));
        let _ = std::fs::remove_file(record);
    }

    #[cfg(unix)]
    #[test]
    fn real_workspace_stacks_panes_and_focuses_vertically() {
        let options = Options {
            cols: 21,
            rows: 7,
            ..Options::default()
        };
        let mut workspace = Workspace::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "printf TOP; cat".to_owned(),
            ],
            None,
            None,
            &options,
        )
        .unwrap();
        workspace.set_selected_shell(vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "printf BOTTOM; cat".to_owned(),
        ]);
        workspace.split(SplitAxis::Rows).unwrap();
        workspace.send(Some(1), b"AGENT\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let frame = loop {
            workspace.pump().unwrap();
            let frame = workspace.frame().unwrap();
            let text = frame.text();
            if text.contains("TOP") && text.contains("BOTTOM") && text.contains("AGENT") {
                break frame;
            }
            assert!(Instant::now() < deadline, "workspace output did not arrive");
            std::thread::sleep(Duration::from_millis(10));
        };

        assert!(frame.text().contains("─────────────────────"));
        assert_eq!(workspace.active_id(), Some(1));
        assert!(workspace.focus_direction(Direction::Up).unwrap());
        assert_eq!(workspace.active_id(), Some(0));
        assert!(workspace.focus_direction(Direction::Down).unwrap());
        assert_eq!(workspace.active_id(), Some(1));
        assert_eq!(
            workspace
                .panes()
                .unwrap()
                .into_iter()
                .map(|pane| (pane.cols, pane.rows))
                .collect::<Vec<_>>(),
            [(21, 2), (21, 3)]
        );
        workspace.resize(21, 9, 9, 18).unwrap();
        assert_eq!(
            workspace
                .panes()
                .unwrap()
                .into_iter()
                .map(|pane| (pane.cols, pane.rows))
                .collect::<Vec<_>>(),
            [(21, 3), (21, 4)]
        );
        workspace.resize(21, 2, 9, 18).unwrap();
        assert_eq!(workspace.status().unwrap().rows, 2);
        workspace.set_grid(1, 2).unwrap();
        assert_eq!(workspace.panes().unwrap()[1].y, 4);
        let constrained = workspace.frame().unwrap();
        assert_eq!(constrained.rows, 2);
        assert!(
            constrained.text().contains("layout too small"),
            "constrained frame: {:?}",
            constrained.text()
        );
        workspace.resize(21, 9, 9, 18).unwrap();
        let restored = workspace.frame().unwrap().text();
        assert!(restored.contains("TOP"));
        assert!(restored.contains("BOTTOM"));
        workspace
            .close_pane(workspace.active_id().unwrap())
            .unwrap();
        let panes = workspace.panes().unwrap();
        assert_eq!(panes.len(), 1);
        assert_eq!((panes[0].cols, panes[0].rows), (21, 8));
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn workspace_retains_final_output_drained_during_exit_observation() {
        let mut workspace = Workspace::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "printf '\\033[5n'; awk 'BEGIN { for (i = 0; i < 500; i++) print \"line-\" i; print \"END-MARKER\" }'"
                    .to_owned(),
            ],
            None,
            None,
            &Options {
                cols: 40,
                rows: 8,
                ..Options::default()
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !workspace.observe_exits().unwrap() {
            workspace.pump().unwrap();
            assert!(Instant::now() < deadline, "workspace command did not exit");
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(workspace.frame().unwrap().text().contains("END-MARKER"));
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn removing_panes_never_observes_a_late_exit_after_composition() {
        let mut workspace = Workspace::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "read line; printf LATE-MARKER".to_owned(),
            ],
            None,
            None,
            &Options {
                cols: 40,
                rows: 8,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(!workspace.observe_exits().unwrap());
        workspace.send(None, b"\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            workspace.pump().unwrap();
            if workspace.frame().unwrap().text().contains("LATE-MARKER") {
                break;
            }
            assert!(Instant::now() < deadline, "workspace output did not arrive");
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(!workspace.remove_observed_exits().unwrap());
        while !workspace.observe_exits().unwrap() {
            assert!(Instant::now() < deadline, "workspace command did not exit");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(workspace.frame().unwrap().text().contains("LATE-MARKER"));
        assert!(workspace.remove_observed_exits().unwrap());
        assert!(workspace.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn targeted_capture_returns_a_panes_final_frame_without_waiting_to_settle() {
        let options = Options {
            cols: 40,
            rows: 8,
            ..Options::default()
        };
        let mut workspace = Workspace::start(
            &["sh".to_owned(), "-c".to_owned(), "cat".to_owned()],
            None,
            None,
            &options,
        )
        .unwrap();
        workspace.set_selected_shell(vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "printf TARGET-FINAL".to_owned(),
        ]);
        workspace.split(SplitAxis::Columns).unwrap();

        let shot = workspace
            .capture(
                Some(1),
                Duration::from_secs(600),
                Duration::from_secs(2),
                |workspace| {
                    workspace.pump()?;
                    workspace.observe_exits()?;
                    Ok(true)
                },
            )
            .unwrap();

        assert!(shot.frame.text().contains("TARGET-FINAL"));
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn composed_capture_returns_a_windows_final_frame_before_removal() {
        let mut workspace = Workspace::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "printf WINDOW-FINAL".to_owned(),
            ],
            None,
            None,
            &Options {
                cols: 40,
                rows: 8,
                ..Options::default()
            },
        )
        .unwrap();
        let mut terminal = WorkspaceTerminal::detached();

        let shot = workspace
            .capture_window(
                "main",
                Duration::from_secs(600),
                Duration::from_secs(2),
                |workspace| terminal.tick(workspace),
            )
            .unwrap();

        assert!(shot.frame.text().contains("WINDOW-FINAL"));
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn failed_hidden_zoom_does_not_change_window_selection() {
        let shell = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let mut workspace = Workspace::start(&shell, None, None, &Options::default()).unwrap();
        workspace
            .create_window(Some("single"), &shell, None)
            .unwrap();
        workspace.select_window("main").unwrap();

        assert!(workspace.toggle_zoom_pane(1).is_err());
        assert_eq!(workspace.active_window_name(), Some("main"));
        assert_eq!(workspace.windows()[1].zoomed_pane, None);
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn failed_attachment_setup_preserves_workspace_presentation() {
        struct FailingWriter;

        impl Write for FailingWriter {
            fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("expected attachment failure"))
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let shell = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let options = Options {
            cols: 80,
            rows: 24,
            ..Options::default()
        };
        let mut workspace = Workspace::start(&shell, None, None, &options).unwrap();
        let mut terminal = WorkspaceTerminal::detached();
        let (_send, receive) = std::sync::mpsc::channel();
        let attached = WorkspaceAttachmentOptions {
            id: 1,
            cols: 120,
            rows: 40,
            cell_width: 10,
            cell_height: 20,
            theme: TerminalTheme::default(),
        };

        assert!(
            terminal
                .attach(&mut workspace, receive, Box::new(FailingWriter), attached,)
                .is_err()
        );
        assert_eq!(
            (workspace.windows()[0].cols, workspace.windows()[0].rows),
            (80, 24)
        );
        assert!(!terminal.is_attached());

        let (_send, receive) = std::sync::mpsc::channel();
        assert!(
            terminal
                .attach(
                    &mut workspace,
                    receive,
                    Box::new(std::io::sink()),
                    WorkspaceAttachmentOptions {
                        id: 2,
                        cols: 120,
                        rows: 1,
                        cell_width: 10,
                        cell_height: 20,
                        theme: TerminalTheme::default(),
                    },
                )
                .is_err()
        );
        assert_eq!((workspace.cols, workspace.rows), (80, 24));
        assert_eq!(workspace.panes().unwrap()[0].rows, 23);

        let (_send, receive) = std::sync::mpsc::channel();
        terminal
            .attach(
                &mut workspace,
                receive,
                Box::new(std::io::sink()),
                WorkspaceAttachmentOptions {
                    id: 3,
                    cols: 100,
                    rows: 30,
                    cell_width: 9,
                    cell_height: 18,
                    theme: TerminalTheme::default(),
                },
            )
            .unwrap();
        assert!(terminal.is_attached());
        workspace.stop();
    }

    #[test]
    fn grid_geometry_places_four_stable_pane_ids_and_recursive_dividers() {
        let layout = grid_layout(&[0, 1, 2, 3], 2, 2).unwrap();
        let geometry = geometry(&layout, 11, 7).unwrap();

        assert_eq!(
            geometry.panes,
            [
                PlacedPane {
                    id: 0,
                    rect: PaneRect {
                        x: 0,
                        y: 0,
                        cols: 5,
                        rows: 3,
                    },
                },
                PlacedPane {
                    id: 1,
                    rect: PaneRect {
                        x: 6,
                        y: 0,
                        cols: 5,
                        rows: 3,
                    },
                },
                PlacedPane {
                    id: 2,
                    rect: PaneRect {
                        x: 0,
                        y: 4,
                        cols: 5,
                        rows: 3,
                    },
                },
                PlacedPane {
                    id: 3,
                    rect: PaneRect {
                        x: 6,
                        y: 4,
                        cols: 5,
                        rows: 3,
                    },
                },
            ]
        );
        assert_eq!(geometry.dividers.len(), 3);
        let composed = compose_workspace(
            11,
            7,
            &geometry,
            &[
                (0, frame(5, 3, "0", Some((0, 0)))),
                (1, frame(5, 3, "1", Some((0, 0)))),
                (2, frame(5, 3, "2", Some((0, 0)))),
                (3, frame(5, 3, "3", Some((2, 1)))),
            ],
            Some(3),
        );
        assert_eq!(
            composed.cursor.as_ref().map(|cursor| (cursor.x, cursor.y)),
            Some((8, 5))
        );
        assert!(composed.text().contains('0'));
        assert!(composed.text().contains('3'));
        let junction = composed
            .cells
            .iter()
            .find(|cell| (cell.x, cell.y) == (5, 3))
            .unwrap();
        assert_eq!(junction.text, "┼");
        assert!(junction.attributes.faint);
    }

    #[cfg(unix)]
    #[test]
    fn named_windows_keep_independent_layouts_and_global_pane_ids() {
        let options = Options {
            cols: 41,
            rows: 13,
            ..Options::default()
        };
        let command = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let mut workspace = Workspace::start(&command, None, None, &options).unwrap();

        assert_eq!(
            workspace
                .windows()
                .iter()
                .map(|window| (window.name.as_str(), window.active_pane))
                .collect::<Vec<_>>(),
            [("main", Some(0))]
        );
        workspace
            .create_window(Some("logs"), &command, None)
            .unwrap();
        workspace.set_grid(2, 1).unwrap();
        assert_eq!(workspace.active_window_name(), Some("logs"));
        assert_eq!(
            workspace
                .panes()
                .unwrap()
                .iter()
                .map(|pane| pane.id)
                .collect::<Vec<_>>(),
            [1, 2]
        );

        workspace.select_window("main").unwrap();
        workspace.send(Some(1), b"HIDDEN\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            workspace.pump().unwrap();
            if workspace.windows[1]
                .shot(Some(1))
                .unwrap()
                .frame
                .text()
                .contains("HIDDEN")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "hidden pane output did not arrive"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(workspace.active_window_name(), Some("main"));
        workspace.set_selected_shell(command.to_vec());
        workspace.split(SplitAxis::Rows).unwrap();
        assert_eq!(
            workspace
                .panes()
                .unwrap()
                .iter()
                .map(|pane| pane.id)
                .collect::<Vec<_>>(),
            [0, 3]
        );
        assert_eq!(workspace.windows()[1].pane_count, 2);

        workspace.rename_window("logs", "output").unwrap();
        assert!(workspace.rename_window("output", "main").is_err());
        workspace.move_pane(3, "output", false).unwrap();
        assert_eq!(
            workspace
                .panes_in(Some("main"))
                .unwrap()
                .iter()
                .map(|pane| pane.id)
                .collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(
            workspace
                .panes_in(Some("output"))
                .unwrap()
                .iter()
                .map(|pane| pane.id)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        workspace.send(Some(3), b"MOVED\n").unwrap();
        assert_eq!(workspace.active_window_name(), Some("main"));
        workspace.close_window("output").unwrap();
        assert_eq!(workspace.windows().len(), 1);
        assert_eq!(workspace.active_window_name(), Some("main"));
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn tab_strip_persists_and_hidden_output_marks_activity_until_selection() {
        let options = Options {
            cols: 41,
            rows: 7,
            ..Options::default()
        };
        let command = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let mut workspace = Workspace::start(&command, None, None, &options).unwrap();

        assert_eq!(workspace.panes().unwrap()[0].rows, 6);
        assert!(workspace.frame().unwrap().text().contains("[0:main]"));
        workspace
            .create_window(Some("logs"), &command, None)
            .unwrap();
        workspace.select_window("main").unwrap();
        workspace.send(Some(1), b"ACTIVITY\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while !workspace.windows()[1].activity {
            workspace.pump().unwrap();
            assert!(
                Instant::now() < deadline,
                "hidden output did not mark activity"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let frame = workspace.frame().unwrap();
        assert!(frame.text().contains("1:logs {+}"));
        let logs = workspace
            .tab_labels(workspace.active_window)
            .into_iter()
            .find(|tab| tab.index == 1)
            .unwrap();
        assert_eq!(workspace.tab_index_at(logs.start, 6), Some(1));
        assert_eq!(
            uncaptured_tab_position(Some((logs.start, 6)), 7, TabPosition::Bottom, false),
            Some((logs.start, 6))
        );
        assert_eq!(
            uncaptured_tab_position(Some((logs.start, 6)), 7, TabPosition::Bottom, true),
            None
        );
        workspace.select_window_index(1).unwrap();
        assert!(!workspace.windows()[1].activity);
        assert!(workspace.frame().unwrap().text().contains("[1:logs]"));
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn tab_badges_compose_activity_kinds() {
        let command = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let mut workspace = Workspace::start(&command, None, None, &Options::default()).unwrap();
        workspace
            .create_window(Some("logs"), &command, None)
            .unwrap();
        workspace.select_window("main").unwrap();
        for kind in [ActivityKind::Output, ActivityKind::Bell, ActivityKind::Exit] {
            workspace.windows[1].mark_activity(kind);
        }
        assert!(workspace.frame().unwrap().text().contains("logs {+!x}"));
        workspace.select_window("logs").unwrap();
        assert!(workspace.windows()[1].activity_kinds.is_empty());
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn hidden_activity_detects_output_bell_and_surviving_pane_exit() {
        let command = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let delayed_activity = [
            "sh".to_owned(),
            "-c".to_owned(),
            r"sleep 0.2; printf '\aOUTPUT'; cat".to_owned(),
        ];
        let mut workspace = Workspace::start(&command, None, None, &Options::default()).unwrap();
        workspace
            .create_window(Some("logs"), &delayed_activity, None)
            .unwrap();
        workspace.select_window("main").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            workspace.pump().unwrap();
            workspace.take_bells();
            let kinds = &workspace.windows[1].activity_kinds;
            if [ActivityKind::Output, ActivityKind::Bell]
                .iter()
                .all(|kind| kinds.contains(kind))
            {
                break;
            }
            assert!(Instant::now() < deadline, "hidden activity did not arrive");
            std::thread::sleep(Duration::from_millis(10));
        }

        let delayed_exit = ["sh".to_owned(), "-c".to_owned(), "sleep 0.2".to_owned()];
        workspace
            .create_window(Some("worker"), &delayed_exit, None)
            .unwrap();
        workspace
            .set_grid_in_with_command(Some("worker"), 1, 2, Some(&command))
            .unwrap();
        workspace.select_window("main").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            workspace.pump().unwrap();
            workspace.observe_exits().unwrap();
            if workspace.windows[2]
                .activity_kinds
                .contains(&ActivityKind::Exit)
            {
                break;
            }
            assert!(Instant::now() < deadline, "hidden exit did not arrive");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(workspace.windows[2].panes.len(), 2);
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn top_tab_strip_offsets_content_geometry_and_mouse_coordinates() {
        let options = Options {
            cols: 41,
            rows: 7,
            ..Options::default()
        };
        let command = [
            "sh".to_owned(),
            "-c".to_owned(),
            "printf CONTENT; cat".to_owned(),
        ];
        let mut workspace = Workspace::start_with_theme(
            &command,
            None,
            None,
            &options,
            TerminalTheme::default(),
            TabPosition::Top,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !workspace.frame().unwrap().text().contains("CONTENT") {
            workspace.pump().unwrap();
            assert!(Instant::now() < deadline, "workspace output did not arrive");
        }

        let frame = workspace.frame().unwrap();
        assert_eq!(frame.text().lines().next(), Some("[0:main]"));
        assert_eq!(workspace.panes().unwrap()[0].y, 1);
        assert_eq!(workspace.tab_index_at(0, 0), Some(0));
        assert_eq!(workspace.pane_at(0, 1), Some((0, 0, 0)));
        assert_eq!(workspace.pane_position(0, 0, 0), Some((0, 0, 0)));
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn runtime_tab_position_reorder_and_context_share_authoritative_state() {
        let command = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let mut workspace = Workspace::start_named_with_theme(
            "identity-test",
            &command,
            None,
            None,
            &Options::default(),
            TerminalTheme::default(),
            TabPosition::Bottom,
        )
        .unwrap();
        workspace
            .create_window(Some("one"), &command, None)
            .unwrap();
        workspace
            .create_window(Some("two"), &command, None)
            .unwrap();
        let two_pane = workspace.windows()[2].active_pane.unwrap();

        workspace.move_window("one", 0).unwrap();
        workspace.move_window("two", 1).unwrap();
        workspace.set_tab_position(TabPosition::Top);

        let windows = workspace.windows();
        assert_eq!(
            windows
                .iter()
                .map(|window| window.name.as_str())
                .collect::<Vec<_>>(),
            ["one", "two", "main"]
        );
        assert!(workspace.move_window("one", 3).is_err());
        let context = workspace.context(Some(two_pane)).unwrap();
        assert_eq!(context.session, "identity-test");
        assert_eq!(context.workspace, "identity-test");
        assert_eq!(context.window, "two");
        assert_eq!(context.window_index, 1);
        assert_eq!(context.pane, two_pane);
        assert_eq!(context.tab_position, TabPosition::Top);
        assert_eq!(workspace.panes_in(Some("two")).unwrap()[0].y, 1);

        workspace.select_window("one").unwrap();
        assert!(workspace.select_previous_window().unwrap());
        assert_eq!(workspace.active_window_name(), Some("two"));
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn pane_environment_identifies_itself_and_context_tracks_window_moves() {
        let command = [
            "sh".to_owned(),
            "-c".to_owned(),
            "printf 'IDENTITY=%s:%s' \"$TERMCTRL_WORKSPACE\" \"$TERMCTRL_PANE_ID\"; cat".to_owned(),
        ];
        let mut workspace = Workspace::start_named_with_theme(
            "agent-home",
            &command,
            None,
            None,
            &Options::default(),
            TerminalTheme::default(),
            TabPosition::Bottom,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            workspace.pump().unwrap();
            if workspace
                .frame()
                .unwrap()
                .text()
                .contains("IDENTITY=agent-home:0")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "pane identity output did not arrive"
            );
        }
        workspace
            .create_window(Some("destination"), &command, None)
            .unwrap();
        workspace.move_pane(0, "destination", false).unwrap();
        let context = workspace.context(Some(0)).unwrap();
        assert_eq!(context.window, "destination");
        assert_eq!(context.pane, 0);
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn command_palette_handles_coalesced_input_and_renders_a_panel() {
        let command = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let mut workspace = Workspace::start(&command, None, None, &Options::default()).unwrap();
        workspace
            .create_window(Some("tests"), &command, None)
            .unwrap();
        workspace.select_window("main").unwrap();
        let mut ui = WorkspaceUi::new();
        ui.open_palette();

        let (action, remainder) = ui.palette_input(&workspace, b"window tests\rtyped after");
        let action = action.unwrap();
        assert_eq!(remainder, b"typed after");
        apply_palette_command(&mut workspace, &mut ui, action).unwrap();
        assert_eq!(workspace.active_window_name(), Some("tests"));

        ui.open_palette();
        let mut decoder = PrefixDecoder::default();
        assert!(decoder.push(b"\x1b").is_empty());
        let actions = decoder.push(b"[B");
        let [InputAction::Send(arrow)] = actions.as_slice() else {
            panic!("fragmented arrow was not normalized into one input action");
        };
        assert_eq!(arrow, b"\x1b[B");
        let (action, remainder) = ui.palette_input(&workspace, arrow);
        assert!(action.is_none());
        assert!(remainder.is_empty());
        assert_eq!(ui.palette.as_ref().unwrap().selected, 1);

        let lines = ui.palette_lines(&workspace).unwrap();
        let mut frame = frame(80, 12, "content", Some((0, 0)));
        add_palette_overlay(&mut frame, &lines, TabPosition::Bottom);
        assert!(frame.text().contains("COMMAND PALETTE"));
        assert!(frame.text().contains("> pane"));
        assert!(frame.cursor.is_none());
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn attachment_resize_recovers_after_an_invalid_transient_size() {
        let command = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let mut workspace = Workspace::start(&command, None, None, &Options::default()).unwrap();
        let mut terminal = WorkspaceTerminal::detached();
        let (_send, receive) = std::sync::mpsc::channel();
        terminal
            .attach(
                &mut workspace,
                receive,
                Box::new(std::io::sink()),
                WorkspaceAttachmentOptions {
                    id: 1,
                    cols: 80,
                    rows: 24,
                    cell_width: 9,
                    cell_height: 18,
                    theme: TerminalTheme::default(),
                },
            )
            .unwrap();

        assert!(
            terminal
                .resize_attachment(&mut workspace, 1, 100, 1, 9, 18)
                .is_err()
        );
        terminal
            .resize_attachment(&mut workspace, 1, 100, 30, 9, 18)
            .unwrap();

        assert_eq!((workspace.cols, workspace.rows), (100, 30));
        assert_eq!(
            (
                workspace.panes().unwrap()[0].cols,
                workspace.panes().unwrap()[0].rows
            ),
            (100, 29)
        );
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn alternate_screen_pane_repaints_its_full_width_after_layout_resizes() {
        let options = Options {
            cols: 81,
            rows: 9,
            ..Options::default()
        };
        let shell = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let redraw = [
            "sh".to_owned(),
            "-c".to_owned(),
            "draw() { set -- $(stty size); printf '\\033[?1049h\\033[2J\\033[%s;1H\\033[7m%*s\\033[0m' \"$1\" \"$2\" RESIZE; }; trap draw WINCH; while :; do draw; sleep 0.05; done"
                .to_owned(),
        ];
        let mut workspace = Workspace::start(&shell, None, None, &options).unwrap();
        workspace
            .set_grid_in_with_command(None, 2, 1, Some(&redraw))
            .unwrap();

        for cols in [101, 61, 121, 81, 101, 61, 121, 81, 101, 61, 121, 81] {
            workspace.resize(cols, 9, 9, 18).unwrap();
            let expected = workspace.panes().unwrap()[1].cols;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                workspace.pump().unwrap();
                let frame = workspace.windows[0].panes[1]
                    .session
                    .current_frame()
                    .unwrap();
                let bottom = frame.rows - 1;
                let painted = frame
                    .cells
                    .iter()
                    .filter(|cell| cell.y == bottom && cell.background != frame.background)
                    .map(|cell| cell.x.saturating_add(cell.width))
                    .max()
                    .unwrap_or(0);
                if frame.cols == expected && frame.text().contains("RESIZE") && painted == expected
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "alternate screen remained partially blank at {expected} columns: painted {painted}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn exited_hidden_window_is_removed_without_changing_selection() {
        let options = Options {
            cols: 41,
            rows: 13,
            ..Options::default()
        };
        let shell = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let mut workspace = Workspace::start(&shell, None, None, &options).unwrap();
        workspace
            .create_window(
                Some("short"),
                &["sh".to_owned(), "-c".to_owned(), "printf DONE".to_owned()],
                None,
            )
            .unwrap();
        workspace.select_window("main").unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            workspace.pump().unwrap();
            if workspace.observe_exits().unwrap() {
                workspace.remove_observed_exits().unwrap();
            }
            if workspace.windows().len() == 1 {
                break;
            }
            assert!(Instant::now() < deadline, "hidden window did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(workspace.active_window_name(), Some("main"));
        assert_eq!(workspace.windows()[0].active_pane, Some(0));
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn semantic_grid_focus_and_close_share_one_layout_tree() {
        let options = Options {
            cols: 41,
            rows: 13,
            ..Options::default()
        };
        let mut workspace = Workspace::start(
            &["sh".to_owned(), "-c".to_owned(), "cat".to_owned()],
            None,
            None,
            &options,
        )
        .unwrap();
        workspace.set_selected_shell(vec!["sh".to_owned(), "-c".to_owned(), "cat".to_owned()]);

        workspace.set_grid(2, 2).unwrap();
        let panes = workspace.panes().unwrap();
        assert_eq!(
            panes.iter().map(|pane| pane.id).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(workspace.active_id(), Some(0));
        assert_eq!(
            panes
                .iter()
                .map(|pane| (pane.x, pane.y, pane.cols, pane.rows))
                .collect::<Vec<_>>(),
            [(0, 0, 20, 5), (21, 0, 20, 5), (0, 6, 20, 6), (21, 6, 20, 6),]
        );
        assert!(workspace.set_grid(2, 1).is_err());
        workspace.focus_pane(3).unwrap();
        assert_eq!(workspace.active_id(), Some(3));
        workspace.focus_pane(0).unwrap();
        assert_eq!(workspace.active_id(), Some(0));
        workspace.focus_pane(3).unwrap();
        assert_eq!(workspace.active_id(), Some(3));
        assert_eq!(workspace.pane_at(22, 8), Some((3, 1, 2)));
        assert_eq!(workspace.pane_at(20, 6), None);
        assert_eq!(workspace.pane_position(0, 22, 8), Some((0, 19, 4)));
        workspace.close_pane(1).unwrap();
        assert_eq!(workspace.active_id(), Some(3));
        assert_eq!(workspace.panes().unwrap().len(), 3);
        assert!(workspace.focus_direction(Direction::Up).unwrap());
        assert_eq!(workspace.active_id(), Some(0));
        workspace.begin_paste().unwrap();
        workspace.close_pane(0).unwrap();
        assert!(workspace.send_paste(b"ignored after close").unwrap());
        assert!(workspace.end_paste().unwrap());
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn pane_resize_and_zoom_preserve_the_recursive_layout() {
        let options = Options {
            cols: 41,
            rows: 13,
            ..Options::default()
        };
        let shell = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let mut workspace = Workspace::start(&shell, None, None, &options).unwrap();
        workspace.set_selected_shell(shell.to_vec());
        workspace.set_grid(2, 2).unwrap();

        workspace.resize_pane(3, Direction::Left, 4).unwrap();
        workspace.resize_pane(3, Direction::Up, 2).unwrap();
        let panes = workspace.panes().unwrap();
        assert_eq!(
            panes
                .iter()
                .map(|pane| (pane.id, pane.x, pane.y, pane.cols, pane.rows))
                .collect::<Vec<_>>(),
            [
                (0, 0, 0, 20, 3),
                (1, 21, 0, 20, 3),
                (2, 0, 4, 16, 8),
                (3, 17, 4, 24, 8),
            ]
        );
        assert!(workspace.resize_pane(3, Direction::Right, 1).is_err());

        workspace.toggle_zoom_pane(3).unwrap();
        assert_eq!(workspace.windows()[0].zoomed_pane, Some(3));
        let panes = workspace.panes().unwrap();
        assert_eq!(
            panes
                .iter()
                .map(|pane| (pane.id, pane.visible, pane.cols, pane.rows))
                .collect::<Vec<_>>(),
            [
                (0, false, 20, 3),
                (1, false, 20, 3),
                (2, false, 16, 8),
                (3, true, 41, 12),
            ]
        );
        assert_eq!(
            (
                workspace.frame().unwrap().cols,
                workspace.frame().unwrap().rows
            ),
            (41, 13)
        );

        workspace.toggle_zoom_pane(3).unwrap();
        assert_eq!(workspace.windows()[0].zoomed_pane, None);
        assert!(workspace.panes().unwrap().iter().all(|pane| pane.visible));
        assert_eq!(
            workspace
                .panes()
                .unwrap()
                .iter()
                .map(|pane| (pane.id, pane.x, pane.y, pane.cols, pane.rows))
                .collect::<Vec<_>>(),
            [
                (0, 0, 0, 20, 3),
                (1, 21, 0, 20, 3),
                (2, 0, 4, 16, 8),
                (3, 17, 4, 24, 8),
            ]
        );
        workspace.stop();
    }

    #[cfg(unix)]
    #[test]
    fn constrained_layout_and_failed_move_preserve_presentation_state() {
        let options = Options {
            cols: 41,
            rows: 13,
            ..Options::default()
        };
        let shell = ["sh".to_owned(), "-c".to_owned(), "cat".to_owned()];
        let mut workspace = Workspace::start(&shell, None, None, &options).unwrap();
        workspace.set_selected_shell(shell.to_vec());
        workspace.set_grid(2, 1).unwrap();
        let mut revisions = Vec::new();
        let before = workspace.active_frame_key(&mut revisions).unwrap();
        workspace.resize(2, 13, 9, 18).unwrap();
        let constrained = workspace.active_frame_key(&mut revisions).unwrap();
        assert_ne!(before, constrained);
        assert!(
            workspace
                .selected_window()
                .unwrap()
                .applied
                .is_constrained()
        );
        workspace.resize(41, 13, 9, 18).unwrap();

        workspace.toggle_zoom_pane(0).unwrap();
        workspace
            .create_window(Some("target"), &shell, None)
            .unwrap();
        workspace.set_selected_shell(shell.to_vec());
        workspace.set_grid(2, 1).unwrap();
        workspace.toggle_zoom_pane(2).unwrap();
        workspace.resize(2, 13, 9, 18).unwrap();

        assert!(workspace.move_pane(1, "target", false).is_err());
        let windows = workspace.windows();
        assert_eq!(windows[0].zoomed_pane, Some(0));
        assert_eq!(windows[1].zoomed_pane, Some(2));
        assert_eq!(windows[0].pane_count, 2);
        assert_eq!(windows[1].pane_count, 2);
        workspace.stop();
    }
}
