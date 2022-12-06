use std::mem::take;

use auto_hash_map::AutoSet;
use once_cell::sync::Lazy;
use parking_lot::{RwLockReadGuard, RwLockWriteGuard};
use turbo_tasks::TaskId;

use super::{PartialTaskState, Task, TaskState, UnloadedTaskState};
use crate::{
    map_guard::{ReadGuard, WriteGuard},
    scope::TaskScopes,
};

pub(super) enum TaskMetaState {
    Full(Box<TaskState>),
    Partial(Box<PartialTaskState>),
    Unloaded(UnloadedTaskState),
}

impl Default for TaskMetaState {
    fn default() -> Self {
        Self::Unloaded(UnloadedTaskState::default())
    }
}

impl TaskMetaState {
    fn into_partial(self) -> Option<PartialTaskState> {
        match self {
            Self::Partial(state) => Some(*state),
            _ => None,
        }
    }

    fn into_unloaded(self) -> Option<UnloadedTaskState> {
        match self {
            Self::Unloaded(state) => Some(state),
            _ => None,
        }
    }

    fn unwrap_full(&self) -> &TaskState {
        match self {
            Self::Full(state) => state,
            _ => panic!("TaskMetaState is not full"),
        }
    }

    fn unwrap_partial(&self) -> &PartialTaskState {
        match self {
            Self::Partial(state) => state,
            _ => panic!("TaskMetaState is not partial"),
        }
    }

    fn unwrap_unloaded(&self) -> &UnloadedTaskState {
        match self {
            Self::Unloaded(state) => state,
            _ => panic!("TaskMetaState is not none"),
        }
    }

    fn unwrap_full_mut(&mut self) -> &mut TaskState {
        match self {
            Self::Full(state) => state,
            _ => panic!("TaskMetaState is not full"),
        }
    }

    fn unwrap_partial_mut(&mut self) -> &mut PartialTaskState {
        match self {
            Self::Partial(state) => state,
            _ => panic!("TaskMetaState is not partial"),
        }
    }

    fn unwrap_unloaded_mut(&mut self) -> &mut UnloadedTaskState {
        match self {
            Self::Unloaded(state) => state,
            _ => panic!("TaskMetaState is not none"),
        }
    }
}

// These need to be impl types since there is no way to reference the zero-sized
// function item type
type TaskMetaStateUnwrapFull = impl Fn(&TaskMetaState) -> &TaskState;
type TaskMetaStateUnwrapPartial = impl Fn(&TaskMetaState) -> &PartialTaskState;
type TaskMetaStateUnwrapUnloaded = impl Fn(&TaskMetaState) -> &UnloadedTaskState;
type TaskMetaStateUnwrapFullMut = impl Fn(&mut TaskMetaState) -> &mut TaskState;
type TaskMetaStateUnwrapPartialMut = impl Fn(&mut TaskMetaState) -> &mut PartialTaskState;
type TaskMetaStateUnwrapUnloadedMut = impl Fn(&mut TaskMetaState) -> &mut UnloadedTaskState;

pub(super) enum TaskMetaStateReadGuard<'a> {
    Full(ReadGuard<'a, TaskMetaState, TaskState, TaskMetaStateUnwrapFull>),
    Partial(ReadGuard<'a, TaskMetaState, PartialTaskState, TaskMetaStateUnwrapPartial>),
    Unloaded(ReadGuard<'a, TaskMetaState, UnloadedTaskState, TaskMetaStateUnwrapUnloaded>),
}

pub(super) type FullTaskWriteGuard<'a> =
    WriteGuard<'a, TaskMetaState, TaskState, TaskMetaStateUnwrapFull, TaskMetaStateUnwrapFullMut>;

pub(super) enum TaskMetaStateWriteGuard<'a> {
    Full(FullTaskWriteGuard<'a>),
    Partial(
        WriteGuard<
            'a,
            TaskMetaState,
            PartialTaskState,
            TaskMetaStateUnwrapPartial,
            TaskMetaStateUnwrapPartialMut,
        >,
    ),
    Unloaded(
        WriteGuard<
            'a,
            TaskMetaState,
            UnloadedTaskState,
            TaskMetaStateUnwrapUnloaded,
            TaskMetaStateUnwrapUnloadedMut,
        >,
    ),
}

impl<'a> From<RwLockReadGuard<'a, TaskMetaState>> for TaskMetaStateReadGuard<'a> {
    fn from(guard: RwLockReadGuard<'a, TaskMetaState>) -> Self {
        match &*guard {
            TaskMetaState::Full(_) => {
                TaskMetaStateReadGuard::Full(ReadGuard::new(guard, TaskMetaState::unwrap_full))
            }
            TaskMetaState::Partial(_) => TaskMetaStateReadGuard::Partial(ReadGuard::new(
                guard,
                TaskMetaState::unwrap_partial,
            )),
            TaskMetaState::Unloaded(_) => TaskMetaStateReadGuard::Unloaded(ReadGuard::new(
                guard,
                TaskMetaState::unwrap_unloaded,
            )),
        }
    }
}

impl<'a> From<RwLockWriteGuard<'a, TaskMetaState>> for TaskMetaStateWriteGuard<'a> {
    fn from(guard: RwLockWriteGuard<'a, TaskMetaState>) -> Self {
        match &*guard {
            TaskMetaState::Full(_) => TaskMetaStateWriteGuard::Full(WriteGuard::new(
                guard,
                TaskMetaState::unwrap_full,
                TaskMetaState::unwrap_full_mut,
            )),
            TaskMetaState::Partial(_) => TaskMetaStateWriteGuard::Partial(WriteGuard::new(
                guard,
                TaskMetaState::unwrap_partial,
                TaskMetaState::unwrap_partial_mut,
            )),
            TaskMetaState::Unloaded(_) => TaskMetaStateWriteGuard::Unloaded(WriteGuard::new(
                guard,
                TaskMetaState::unwrap_unloaded,
                TaskMetaState::unwrap_unloaded_mut,
            )),
        }
    }
}

impl<'a> TaskMetaStateWriteGuard<'a> {
    pub(super) fn full_from(
        mut guard: RwLockWriteGuard<'a, TaskMetaState>,
        task: &Task,
    ) -> FullTaskWriteGuard<'a> {
        match &*guard {
            TaskMetaState::Full(_) => {}
            TaskMetaState::Partial(_) => {
                let partial = take(&mut *guard).into_partial().unwrap();
                *guard = TaskMetaState::Full(box partial.into_full());
            }
            TaskMetaState::Unloaded(_) => {
                let unloaded = take(&mut *guard).into_unloaded().unwrap();
                *guard = TaskMetaState::Full(box unloaded.into_full(task.id));
            }
        }
        WriteGuard::new(
            guard,
            TaskMetaState::unwrap_full,
            TaskMetaState::unwrap_full_mut,
        )
    }

    #[allow(dead_code, reason = "We need this in future")]
    pub(super) fn partial_from(
        mut guard: RwLockWriteGuard<'a, TaskMetaState>,
        task: &Task,
    ) -> Self {
        match &*guard {
            TaskMetaState::Full(_) => TaskMetaStateWriteGuard::Full(WriteGuard::new(
                guard,
                TaskMetaState::unwrap_full,
                TaskMetaState::unwrap_full_mut,
            )),
            TaskMetaState::Partial(_) => TaskMetaStateWriteGuard::Partial(WriteGuard::new(
                guard,
                TaskMetaState::unwrap_partial,
                TaskMetaState::unwrap_partial_mut,
            )),
            TaskMetaState::Unloaded(_) => {
                let unloaded = take(&mut *guard).into_unloaded().unwrap();
                *guard = TaskMetaState::Partial(box unloaded.into_partial(task.id));
                TaskMetaStateWriteGuard::Partial(WriteGuard::new(
                    guard,
                    TaskMetaState::unwrap_partial,
                    TaskMetaState::unwrap_partial_mut,
                ))
            }
        }
    }

    pub(super) fn scopes_and_children(&mut self) -> (&mut TaskScopes, &AutoSet<TaskId>) {
        match self {
            TaskMetaStateWriteGuard::Full(state) => {
                let TaskState {
                    ref mut scopes,
                    ref children,
                    ..
                } = **state;
                (scopes, children)
            }
            TaskMetaStateWriteGuard::Partial(state) => {
                let PartialTaskState { ref mut scopes, .. } = **state;
                static EMPTY: Lazy<AutoSet<TaskId>> = Lazy::new(|| AutoSet::new());
                (scopes, &*EMPTY)
            }
            TaskMetaStateWriteGuard::Unloaded(_) => unreachable!(
                "TaskMetaStateWriteGuard::scopes_and_children must be called with at least a \
                 partial state"
            ),
        }
    }
}
