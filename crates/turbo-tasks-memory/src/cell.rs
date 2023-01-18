use std::{
    fmt::Debug,
    mem::{replace, take},
};

use auto_hash_map::AutoSet;
use turbo_tasks::{
    backend::CellContent,
    event::{Event, EventListener},
    TaskId, TurboTasksBackendApi,
};

#[derive(Default, Debug)]
pub(crate) enum Cell {
    /// No content has been set yet, or it was removed for memory pressure
    /// reasons.
    /// Assigning a value will transition to the Value state.
    /// Reading this cell will transition to the Recomputing state.
    #[default]
    Empty,
    /// The content has been removed for memory pressure reasons, but the
    /// tracking is still active. Any update will invalidate dependent tasks.
    /// Assigning a value will transition to the Value state.
    /// Reading this cell will transition to the Recomputing state.
    TrackedValueless { dependent_tasks: AutoSet<TaskId> },
    /// Someone wanted to read the content and it was not available. The content
    /// is now being recomputed.
    /// Assigning a value will transition to the Value state.
    Recomputing {
        dependent_tasks: AutoSet<TaskId>,
        event: Event,
    },
    /// The content was set only once and is tracked.
    /// GC operation will transition to the TrackedValueless state.
    Value {
        dependent_tasks: AutoSet<TaskId>,
        content: CellContent,
    },
}

#[derive(Debug)]
pub struct RecomputingCell {
    pub listener: EventListener,
    pub schedule: bool,
}

impl Cell {
    pub fn has_value(&self) -> bool {
        match self {
            Cell::Empty => false,
            Cell::Recomputing { .. } => false,
            Cell::TrackedValueless { .. } => false,
            Cell::Value { .. } => true,
        }
    }

    pub fn remove_dependent_task(&mut self, task: TaskId) {
        match self {
            Cell::Empty => {}
            Cell::Value {
                dependent_tasks, ..
            }
            | Cell::TrackedValueless {
                dependent_tasks, ..
            }
            | Cell::Recomputing {
                dependent_tasks, ..
            } => {
                dependent_tasks.remove(&task);
            }
        }
    }

    pub fn has_dependent_tasks(&self) -> bool {
        match self {
            Cell::Empty => false,
            Cell::Recomputing {
                dependent_tasks, ..
            }
            | Cell::Value {
                dependent_tasks, ..
            }
            | Cell::TrackedValueless {
                dependent_tasks, ..
            } => !dependent_tasks.is_empty(),
        }
    }

    pub fn dependent_tasks(&self) -> impl Iterator<Item = TaskId> + '_ {
        match self {
            Cell::Empty => {
                static EMPTY: AutoSet<TaskId> = AutoSet::new();
                EMPTY.iter().copied()
            }
            Cell::Value {
                dependent_tasks, ..
            }
            | Cell::TrackedValueless {
                dependent_tasks, ..
            }
            | Cell::Recomputing {
                dependent_tasks, ..
            } => dependent_tasks.iter().copied(),
        }
    }

    fn recompute(
        &mut self,
        dependent_tasks: AutoSet<TaskId>,
        description: impl Fn() -> String + Sync + Send + 'static,
        note: impl Fn() -> String + Sync + Send + 'static,
    ) -> EventListener {
        let event = Event::new(move || (description)() + " -> Cell::Recomputing::event");
        let listener = event.listen_with_note(note);
        *self = Cell::Recomputing {
            event,
            dependent_tasks,
        };
        listener
    }

    pub fn read_content(
        &mut self,
        reader: TaskId,
        description: impl Fn() -> String + Sync + Send + 'static,
        note: impl Fn() -> String + Sync + Send + 'static,
    ) -> Result<CellContent, RecomputingCell> {
        match self {
            Cell::Empty => {
                let listener = self.recompute(AutoSet::new(), description, note);
                Err(RecomputingCell {
                    listener,
                    schedule: true,
                })
            }
            Cell::Recomputing { event, .. } => {
                let listener = event.listen_with_note(note);
                Err(RecomputingCell {
                    listener,
                    schedule: false,
                })
            }
            &mut Cell::TrackedValueless {
                ref mut dependent_tasks,
            } => {
                let dependent_tasks = take(dependent_tasks);
                let listener = self.recompute(dependent_tasks, description, note);
                Err(RecomputingCell {
                    listener,
                    schedule: true,
                })
            }
            Cell::Value {
                content,
                dependent_tasks,
                ..
            } => {
                dependent_tasks.insert(reader);
                Ok(content.clone())
            }
        }
    }

    /// INVALIDATION: Be careful with this, it will not track dependencies, so
    /// using it could break cache invalidation.
    pub fn read_content_untracked(
        &mut self,
        description: impl Fn() -> String + Sync + Send + 'static,
        note: impl Fn() -> String + Sync + Send + 'static,
    ) -> Result<CellContent, RecomputingCell> {
        match self {
            Cell::Empty => {
                let listener = self.recompute(AutoSet::new(), description, note);
                Err(RecomputingCell {
                    listener,
                    schedule: true,
                })
            }
            Cell::Recomputing { event, .. } => {
                let listener = event.listen_with_note(note);
                Err(RecomputingCell {
                    listener,
                    schedule: false,
                })
            }
            &mut Cell::TrackedValueless {
                ref mut dependent_tasks,
            } => {
                let dependent_tasks = take(dependent_tasks);
                let listener = self.recompute(dependent_tasks, description, note);
                Err(RecomputingCell {
                    listener,
                    schedule: true,
                })
            }
            Cell::Value { content, .. } => Ok(content.clone()),
        }
    }

    /// INVALIDATION: Be careful with this, it will not track dependencies, so
    /// using it could break cache invalidation.
    pub fn read_own_content_untracked(&self) -> CellContent {
        match self {
            Cell::Empty | Cell::Recomputing { .. } | Cell::TrackedValueless { .. } => {
                CellContent(None)
            }
            Cell::Value { content, .. } => content.clone(),
        }
    }

    pub fn assign(&mut self, content: CellContent, turbo_tasks: &dyn TurboTasksBackendApi) {
        match self {
            Cell::Empty => {
                *self = Cell::Value {
                    content,
                    dependent_tasks: AutoSet::new(),
                };
            }
            &mut Cell::Recomputing {
                ref mut event,
                ref mut dependent_tasks,
            } => {
                event.notify(usize::MAX);
                *self = Cell::Value {
                    content,
                    dependent_tasks: take(dependent_tasks),
                };
            }
            &mut Cell::TrackedValueless {
                ref mut dependent_tasks,
            } => {
                // Assigning to a cell will invalidate all dependent tasks as the content might
                // have changed.
                if !dependent_tasks.is_empty() {
                    turbo_tasks.schedule_notify_tasks_set(dependent_tasks);
                }
                *self = Cell::Value {
                    content,
                    dependent_tasks: AutoSet::new(),
                };
            }
            Cell::Value {
                content: ref mut cell_content,
                dependent_tasks,
            } => {
                if content != *cell_content {
                    if !dependent_tasks.is_empty() {
                        turbo_tasks.schedule_notify_tasks_set(dependent_tasks);
                        dependent_tasks.clear();
                    }
                    *cell_content = content;
                }
            }
        }
    }

    pub fn shrink_to_fit(&mut self) {
        match self {
            Cell::Empty => {}
            Cell::TrackedValueless {
                dependent_tasks, ..
            }
            | Cell::Recomputing {
                dependent_tasks, ..
            }
            | Cell::Value {
                dependent_tasks, ..
            } => {
                dependent_tasks.shrink_to_fit();
            }
        }
    }

    /// Takes the content out of the cell. Make sure to drop the content outside
    /// of the task state lock.
    #[must_use]
    pub fn gc_content(&mut self) -> Option<CellContent> {
        match self {
            Cell::Empty | Cell::Recomputing { .. } | Cell::TrackedValueless { .. } => None,
            Cell::Value {
                dependent_tasks, ..
            } => {
                let dependent_tasks = take(dependent_tasks);
                let Cell::Value { content, .. } = replace(self, Cell::TrackedValueless {
                    dependent_tasks,
                }) else { unreachable!() };
                Some(content)
            }
        }
    }

    pub fn gc_drop(self, turbo_tasks: &dyn TurboTasksBackendApi) {
        match self {
            Cell::Empty => {}
            Cell::Recomputing {
                event,
                dependent_tasks,
                ..
            } => {
                event.notify(usize::MAX);
                if !dependent_tasks.is_empty() {
                    turbo_tasks.schedule_notify_tasks_set(&dependent_tasks);
                }
            }
            Cell::TrackedValueless {
                dependent_tasks, ..
            }
            | Cell::Value {
                dependent_tasks, ..
            } => {
                if !dependent_tasks.is_empty() {
                    turbo_tasks.schedule_notify_tasks_set(&dependent_tasks);
                }
            }
        }
    }
}
