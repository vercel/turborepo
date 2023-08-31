use std::{
    hash::{BuildHasher, Hash},
    mem::take,
};

use auto_hash_map::{map::Entry, AutoMap, AutoSet};
use nohash_hasher::BuildNoHashHasher;
use parking_lot::Mutex;
use turbo_tasks::{event::Event, RawVc, TaskId, TraitTypeId, TurboTasksBackendApi};

use super::{meta_state::TaskMetaStateWriteGuard, TaskStateType};
use crate::{
    aggregation_tree::{AggregationContext, AggregationItemLock, AggregationTreeLeaf},
    MemoryBackend,
};

pub enum RootType {
    Once,
    Root,
}

#[derive(Debug, Default)]
pub struct CollectiblesInfo {
    collectibles: AutoMap<RawVc, i32>,
    dependent_tasks: AutoSet<TaskId, BuildNoHashHasher<TaskId>>,
}

impl CollectiblesInfo {
    fn is_unset(&self) -> bool {
        self.collectibles.is_empty() && self.dependent_tasks.is_empty()
    }
}

pub enum RootInfoType {
    IsActive,
}

pub struct Aggregated {
    /// The number of unfinished items in the lower aggregation level.
    /// Unfinished means not "Done".
    // TODO determine if this can go negative in concurrent situations.
    pub unfinished: i32,
    /// Event that will be notified when all unfinished tasks are done.
    pub unfinished_event: Event,
    /// A list of all tasks that are dirty.
    /// When the it becomes active, these need to be scheduled.
    // TODO evaluate a more efficient data structure for this since we are copying the list on
    // every level.
    pub dirty_tasks: AutoMap<TaskId, i32, BuildNoHashHasher<TaskId>>,
    /// Emitted collectibles with count and dependent_tasks by trait type
    pub collectibles: AutoMap<TraitTypeId, CollectiblesInfo, BuildNoHashHasher<TraitTypeId>>,

    /// Only used for the aggregation root. Which kind of root is this?
    /// [RootType::Once] for OnceTasks or [RootType::Root] for Root Tasks.
    /// It's set to None for other tasks, when the once task is done or when the
    /// root task is disposed.
    pub root_type: Option<RootType>,
}

impl Default for Aggregated {
    fn default() -> Self {
        Self {
            unfinished: 0,
            unfinished_event: Event::new(|| "Aggregated::unfinished_event".to_string()),
            dirty_tasks: AutoMap::with_hasher(),
            collectibles: AutoMap::with_hasher(),
            root_type: None,
        }
    }
}

impl Aggregated {
    pub(crate) fn remove_collectible_dependent_task(
        &mut self,
        trait_type: TraitTypeId,
        reader: TaskId,
    ) {
        if let Entry::Occupied(mut entry) = self.collectibles.entry(trait_type) {
            let info = entry.get_mut();
            info.dependent_tasks.remove(&reader);
            if info.is_unset() {
                entry.remove();
            }
        }
    }

    pub(crate) fn read_collectibles(
        &mut self,
        trait_type: TraitTypeId,
        reader: TaskId,
    ) -> AutoMap<RawVc, i32> {
        match self.collectibles.entry(trait_type) {
            Entry::Occupied(mut e) => {
                let info = e.get_mut();
                info.dependent_tasks.insert(reader);
                info.collectibles.clone()
            }
            Entry::Vacant(e) => {
                e.insert(CollectiblesInfo::default())
                    .dependent_tasks
                    .insert(reader);
                AutoMap::default()
            }
        }
    }
}

#[derive(Default)]
pub struct TaskChange {
    pub unfinished: i32,
    pub dirty_tasks_update: Vec<(TaskId, i32)>,
    pub collectibles: Vec<(TraitTypeId, RawVc, i32)>,
}

impl TaskChange {
    fn is_empty(&self) -> bool {
        self.unfinished == 0 && self.dirty_tasks_update.is_empty() && self.collectibles.is_empty()
    }
}

pub struct TaskAggregationContext<'a> {
    pub turbo_tasks: &'a dyn TurboTasksBackendApi<MemoryBackend>,
    pub backend: &'a MemoryBackend,
    pub dirty_tasks_to_schedule: Mutex<Option<AutoSet<TaskId, BuildNoHashHasher<TaskId>>>>,
}

impl<'a> TaskAggregationContext<'a> {
    pub fn new(
        turbo_tasks: &'a dyn TurboTasksBackendApi<MemoryBackend>,
        backend: &'a MemoryBackend,
    ) -> Self {
        Self {
            turbo_tasks,
            backend,
            dirty_tasks_to_schedule: Mutex::new(None),
        }
    }

    pub fn take_scheduled_dirty_task(&mut self, task: TaskId) -> bool {
        let dirty_task_to_schedule = self.dirty_tasks_to_schedule.get_mut();
        dirty_task_to_schedule
            .as_mut()
            .map(|t| t.remove(&task))
            .unwrap_or(false)
    }

    pub fn schedule_dirty_tasks_if_needed(&mut self) {
        let tasks = self.dirty_tasks_to_schedule.get_mut();
        if let Some(tasks) = tasks.as_mut() {
            let tasks = take(tasks);
            if !tasks.is_empty() {
                self.backend
                    .schedule_when_dirty_from_aggregation(tasks, self.turbo_tasks);
            }
        }
    }
}

#[cfg(debug_assertions)]
impl<'a> Drop for TaskAggregationContext<'a> {
    fn drop(&mut self) {
        let tasks_to_schedule = self.dirty_tasks_to_schedule.get_mut();
        if let Some(tasks_to_schedule) = tasks_to_schedule.as_ref() {
            if !tasks_to_schedule.is_empty() {
                panic!("TaskAggregationContext dropped without scheduling all tasks");
            }
        }
    }
}

impl<'a> AggregationContext for TaskAggregationContext<'a> {
    type ItemLock<'l> = TaskGuard<'l> where Self: 'l;
    type Info = Aggregated;
    type ItemChange = TaskChange;
    type ItemRef = TaskId;
    type RootInfo = bool;
    type RootInfoType = RootInfoType;

    fn is_blue(&self, reference: TaskId) -> bool {
        self.backend.with_task(reference, |task| task.is_blue())
    }

    fn item(&self, reference: TaskId) -> Self::ItemLock<'_> {
        TaskGuard {
            id: reference,
            guard: self.backend.task(reference).state_mut(),
        }
    }

    fn apply_change(
        &self,
        info: &mut Aggregated,
        change: &Self::ItemChange,
    ) -> Option<Self::ItemChange> {
        info.unfinished += change.unfinished;
        for &(task, count) in change.dirty_tasks_update.iter() {
            let value = update_count_entry(info.dirty_tasks.entry(task), count);
            if value > 0 {
                if matches!(info.root_type, Some(RootType::Root) | Some(RootType::Once)) {
                    let mut tasks_to_schedule = self.dirty_tasks_to_schedule.lock();
                    tasks_to_schedule.get_or_insert_default().insert(task);
                }
            }
        }
        for &(trait_type_id, collectible, count) in change.collectibles.iter() {
            let collectibles_info_entry = info.collectibles.entry(trait_type_id);
            match collectibles_info_entry {
                Entry::Occupied(mut e) => {
                    let collectibles_info = e.get_mut();
                    update_count_entry(collectibles_info.collectibles.entry(collectible), count);
                    if collectibles_info.is_unset() {
                        e.remove();
                    }
                }
                Entry::Vacant(e) => {
                    let mut collectibles_info = CollectiblesInfo::default();
                    update_count_entry(collectibles_info.collectibles.entry(collectible), count);
                    e.insert(collectibles_info);
                }
            }
        }
        let new_change = TaskChange {
            unfinished: info.unfinished.clamp(0, 1),
            dirty_tasks_update: change.dirty_tasks_update.clone(),
            collectibles: change.collectibles.clone(),
        };
        if new_change.is_empty() {
            None
        } else {
            Some(new_change)
        }
    }

    fn info_to_add_change(&self, info: &Aggregated) -> Option<Self::ItemChange> {
        let mut change = TaskChange {
            unfinished: 0,
            dirty_tasks_update: vec![],
            collectibles: vec![],
        };
        if info.unfinished > 0 {
            change.unfinished = 1;
        }
        for (&task, &count) in info.dirty_tasks.iter() {
            change.dirty_tasks_update.push((task, count));
        }
        for (trait_type_id, collectibles_info) in info.collectibles.iter() {
            for (collectible, count) in collectibles_info.collectibles.iter() {
                change
                    .collectibles
                    .push((*trait_type_id, *collectible, *count));
            }
        }
        if change.is_empty() {
            None
        } else {
            Some(change)
        }
    }

    fn info_to_remove_change(&self, info: &Aggregated) -> Option<Self::ItemChange> {
        let mut change = TaskChange {
            unfinished: 0,
            dirty_tasks_update: vec![],
            collectibles: vec![],
        };
        if info.unfinished > 0 {
            change.unfinished = -1;
        }
        for (&task, &count) in info.dirty_tasks.iter() {
            change.dirty_tasks_update.push((task, -count));
        }
        for (trait_type_id, collectibles_info) in info.collectibles.iter() {
            for (collectible, count) in collectibles_info.collectibles.iter() {
                change
                    .collectibles
                    .push((*trait_type_id, *collectible, -*count));
            }
        }
        if change.is_empty() {
            None
        } else {
            Some(change)
        }
    }

    fn new_root_info(&self, _root_info_type: &RootInfoType) -> Self::RootInfo {
        false
    }

    fn info_to_root_info(
        &self,
        info: &Aggregated,
        root_info_type: &RootInfoType,
    ) -> Self::RootInfo {
        match root_info_type {
            RootInfoType::IsActive => info.root_type.is_some(),
        }
    }

    fn merge_root_info(
        &self,
        root_info: &mut Self::RootInfo,
        other: Self::RootInfo,
    ) -> std::ops::ControlFlow<()> {
        if other {
            *root_info = true;
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    }
}

pub struct TaskGuard<'l> {
    pub(super) id: TaskId,
    pub(super) guard: TaskMetaStateWriteGuard<'l>,
}

impl<'l> AggregationItemLock for TaskGuard<'l> {
    type Info = Aggregated;
    type ItemRef = TaskId;
    type ItemChange = TaskChange;
    type ChildrenIter<'a> = impl Iterator<Item = TaskId> + 'a where Self: 'a;

    fn leaf(&mut self) -> &mut AggregationTreeLeaf<Self::Info, Self::ItemRef> {
        self.guard.ensure_at_least_partial();
        match self.guard {
            TaskMetaStateWriteGuard::Full(ref mut guard) => &mut guard.aggregation_leaf,
            TaskMetaStateWriteGuard::Partial(ref mut guard) => &mut guard.aggregation_leaf,
            TaskMetaStateWriteGuard::Unloaded(_) => unreachable!(),
        }
    }

    fn children(&self) -> Self::ChildrenIter<'_> {
        match self.guard {
            TaskMetaStateWriteGuard::Full(ref guard) => {
                Some(guard.children.iter().copied()).into_iter().flatten()
            }
            TaskMetaStateWriteGuard::Partial(_) | TaskMetaStateWriteGuard::Unloaded(_) => {
                None.into_iter().flatten()
            }
        }
    }

    fn is_blue(&self) -> bool {
        false
    }

    fn get_add_change(&self) -> Option<Self::ItemChange> {
        match self.guard {
            TaskMetaStateWriteGuard::Full(ref guard) => {
                let mut change = TaskChange::default();
                if !matches!(
                    guard.state_type,
                    TaskStateType::Done { .. }
                        | TaskStateType::InProgress {
                            count_as_finished: true,
                            ..
                        }
                ) {
                    change.unfinished = 1;
                }
                if matches!(guard.state_type, TaskStateType::Dirty { .. }) {
                    change.dirty_tasks_update.push((self.id, 1));
                }
                // TODO collectibles
                if change.is_empty() {
                    None
                } else {
                    Some(change)
                }
            }
            TaskMetaStateWriteGuard::Partial(_) | TaskMetaStateWriteGuard::Unloaded(_) => None,
        }
    }

    fn get_remove_change(&self) -> Option<Self::ItemChange> {
        match self.guard {
            TaskMetaStateWriteGuard::Full(ref guard) => {
                let mut change = TaskChange::default();
                if !matches!(
                    guard.state_type,
                    TaskStateType::Done { .. }
                        | TaskStateType::InProgress {
                            count_as_finished: true,
                            ..
                        }
                ) {
                    change.unfinished = -1;
                }
                if matches!(guard.state_type, TaskStateType::Dirty { .. }) {
                    change.dirty_tasks_update.push((self.id, -1));
                }
                // TODO collectibles
                if change.is_empty() {
                    None
                } else {
                    Some(change)
                }
            }
            TaskMetaStateWriteGuard::Partial(_) | TaskMetaStateWriteGuard::Unloaded(_) => None,
        }
    }
}

pub type TaskAggregationTreeLeaf = AggregationTreeLeaf<Aggregated, TaskId>;

fn update_count_entry<K: Eq + Hash, H: BuildHasher + Default>(
    entry: Entry<'_, K, i32, H>,
    update: i32,
) -> i32 {
    match entry {
        Entry::Occupied(mut e) => {
            let value = e.get_mut();
            *value += update;
            if *value == 0 {
                e.remove();
                0
            } else {
                *value
            }
        }
        Entry::Vacant(e) => {
            e.insert(update);
            update
        }
    }
}
