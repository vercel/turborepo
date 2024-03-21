use std::{hash::BuildHasherDefault, mem::take};

use auto_hash_map::{map::Entry, AutoMap, AutoSet};
use nohash_hasher::BuildNoHashHasher;
use rustc_hash::FxHasher;
use turbo_tasks::{CellId, TaskId, TraitTypeId, ValueTypeId};

#[derive(Hash, Copy, Clone, PartialEq, Eq)]
pub enum TaskDependency {
    Output(TaskId),
    Cell(TaskId, CellId),
    Collectibles(TaskId, TraitTypeId),
}

enum TaskDependencyData {
    Inline(TaskDependencyDataInline),
    InlineAndOutput(TaskDependencyDataInline),
    Boxed(TaskDependencyDataBoxed),
    BoxedAndOutput(TaskDependencyDataBoxed),
}

#[derive(Copy, Clone)]
struct TaskDependencyDataInline {
    cell_zero: Option<ValueTypeId>,
}

struct TaskDependencyDataBoxed {
    dependencies: Box<AutoSet<DependencyType, BuildHasherDefault<FxHasher>, 1>>,
}

#[derive(Hash, Copy, Clone, PartialEq, Eq)]
enum DependencyType {
    Cell(CellId),
    Collectibles(TraitTypeId),
}

impl DependencyType {
    fn as_task_dependency(&self, task: TaskId) -> TaskDependency {
        match self {
            DependencyType::Cell(cell) => TaskDependency::Cell(task, *cell),
            DependencyType::Collectibles(trait_type) => {
                TaskDependency::Collectibles(task, *trait_type)
            }
        }
    }
}

impl TaskDependencyData {
    pub fn insert(&mut self, dependency: TaskDependency) {
        match self {
            TaskDependencyData::Inline(data) => {
                let new_dep = match dependency {
                    TaskDependency::Output(_) => {
                        *self = TaskDependencyData::InlineAndOutput(*data);
                        return;
                    }
                    TaskDependency::Cell(_, cell_id) if cell_id.index == 0 => {
                        if let Some(type_id) = data.cell_zero {
                            if type_id == cell_id.type_id {
                                return;
                            }
                            DependencyType::Cell(cell_id)
                        } else {
                            data.cell_zero = Some(cell_id.type_id);
                            return;
                        }
                    }
                    TaskDependency::Cell(_, cell_id) => DependencyType::Cell(cell_id),
                    TaskDependency::Collectibles(_, trait_id) => {
                        DependencyType::Collectibles(trait_id)
                    }
                };
                if let Some(type_id) = data.cell_zero {
                    *self = TaskDependencyData::Boxed(TaskDependencyDataBoxed {
                        dependencies: Box::new(AutoSet::from([
                            DependencyType::Cell(CellId { type_id, index: 0 }),
                            new_dep,
                        ])),
                    });
                } else {
                    *self = TaskDependencyData::Boxed(TaskDependencyDataBoxed {
                        dependencies: Box::new(AutoSet::from([new_dep])),
                    });
                }
            }
            TaskDependencyData::InlineAndOutput(data) => {
                let new_dep = match dependency {
                    TaskDependency::Output(_) => {
                        return;
                    }
                    TaskDependency::Cell(_, cell_id) if cell_id.index == 0 => {
                        if let Some(type_id) = data.cell_zero {
                            if type_id == cell_id.type_id {
                                return;
                            }
                            DependencyType::Cell(cell_id)
                        } else {
                            data.cell_zero = Some(cell_id.type_id);
                            return;
                        }
                    }
                    TaskDependency::Cell(_, cell_id) => DependencyType::Cell(cell_id),
                    TaskDependency::Collectibles(_, trait_id) => {
                        DependencyType::Collectibles(trait_id)
                    }
                };
                if let Some(type_id) = data.cell_zero {
                    *self = TaskDependencyData::BoxedAndOutput(TaskDependencyDataBoxed {
                        dependencies: Box::new(AutoSet::from([
                            DependencyType::Cell(CellId { type_id, index: 0 }),
                            new_dep,
                        ])),
                    });
                } else {
                    *self = TaskDependencyData::BoxedAndOutput(TaskDependencyDataBoxed {
                        dependencies: Box::new(AutoSet::from([new_dep])),
                    });
                }
            }
            TaskDependencyData::Boxed(data) => match dependency {
                TaskDependency::Output(_) => {
                    *self = TaskDependencyData::BoxedAndOutput(TaskDependencyDataBoxed {
                        dependencies: take(&mut data.dependencies),
                    });
                }
                TaskDependency::Cell(_, cell_id) => {
                    data.dependencies.insert(DependencyType::Cell(cell_id));
                }
                TaskDependency::Collectibles(_, trait_id) => {
                    data.dependencies
                        .insert(DependencyType::Collectibles(trait_id));
                }
            },
            TaskDependencyData::BoxedAndOutput(data) => match dependency {
                TaskDependency::Output(_) => {}
                TaskDependency::Cell(_, cell_id) => {
                    data.dependencies.insert(DependencyType::Cell(cell_id));
                }
                TaskDependency::Collectibles(_, trait_id) => {
                    data.dependencies
                        .insert(DependencyType::Collectibles(trait_id));
                }
            },
        }
    }
}

enum TaskDependencyDataIter<'a> {
    Data {
        task: TaskId,
        output: bool,
        cell_zero: bool,
        data: &'a TaskDependencyData,
    },
    Iter(TaskId, auto_hash_map::set::Iter<'a, DependencyType>),
}

impl<'a> Iterator for TaskDependencyDataIter<'a> {
    type Item = TaskDependency;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            TaskDependencyDataIter::Data {
                task,
                output,
                cell_zero,
                data,
            } => match data {
                TaskDependencyData::InlineAndOutput(data) => {
                    if !*output {
                        *output = true;
                        return Some(TaskDependency::Output(*task));
                    }
                    if let Some(type_id) = data.cell_zero {
                        if !*cell_zero {
                            *cell_zero = true;
                            return Some(TaskDependency::Cell(*task, CellId { type_id, index: 0 }));
                        }
                    }
                    None
                }
                TaskDependencyData::Inline(data) => {
                    if let Some(type_id) = data.cell_zero {
                        if !*cell_zero {
                            *cell_zero = true;
                            return Some(TaskDependency::Cell(*task, CellId { type_id, index: 0 }));
                        }
                    }
                    None
                }
                TaskDependencyData::BoxedAndOutput(data) => {
                    if !*output {
                        *output = true;
                        return Some(TaskDependency::Output(*task));
                    }
                    let mut iter = data.dependencies.iter();
                    let next = iter.next();
                    let next = next.map(|dependency| dependency.as_task_dependency(*task));
                    *self = TaskDependencyDataIter::Iter(*task, iter);
                    next
                }
                TaskDependencyData::Boxed(data) => {
                    let mut iter = data.dependencies.iter();
                    let next = iter.next();
                    let next = next.map(|dependency| dependency.as_task_dependency(*task));
                    *self = TaskDependencyDataIter::Iter(*task, iter);
                    next
                }
            },
            TaskDependencyDataIter::Iter(task, iter) => iter
                .next()
                .map(|dependency| dependency.as_task_dependency(*task)),
        }
    }
}

enum TaskDependencyDataIntoIter {
    Data {
        task: TaskId,
        data: TaskDependencyData,
    },
    Iter(TaskId, auto_hash_map::set::IntoIter<DependencyType, 1>),
}

impl Iterator for TaskDependencyDataIntoIter {
    type Item = TaskDependency;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            TaskDependencyDataIntoIter::Data {
                task,
                data: outer_data,
            } => match outer_data {
                TaskDependencyData::InlineAndOutput(data) => {
                    *outer_data = TaskDependencyData::Inline(*data);
                    Some(TaskDependency::Output(*task))
                }
                TaskDependencyData::Inline(data) => data
                    .cell_zero
                    .take()
                    .map(|type_id| TaskDependency::Cell(*task, CellId { type_id, index: 0 })),
                TaskDependencyData::BoxedAndOutput(data) => {
                    *outer_data = TaskDependencyData::Boxed(TaskDependencyDataBoxed {
                        dependencies: take(&mut data.dependencies),
                    });
                    Some(TaskDependency::Output(*task))
                }
                TaskDependencyData::Boxed(data) => {
                    let mut iter = take(&mut data.dependencies).into_iter();
                    let next = iter
                        .next()
                        .map(|dependency| dependency.as_task_dependency(*task));
                    *self = TaskDependencyDataIntoIter::Iter(*task, iter);
                    next
                }
            },
            TaskDependencyDataIntoIter::Iter(task, iter) => iter
                .next()
                .map(|dependency| dependency.as_task_dependency(*task)),
        }
    }
}

pub struct TaskDependencies {
    dependencies: AutoMap<TaskId, TaskDependencyData, BuildNoHashHasher<TaskId>>,
}

impl Default for TaskDependencies {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskDependencies {
    pub fn new() -> Self {
        Self {
            dependencies: AutoMap::with_hasher(),
        }
    }

    pub fn shrink_to_fit(&mut self) {
        self.dependencies.shrink_to_fit();
    }

    pub fn is_empty(&self) -> bool {
        self.dependencies.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = TaskDependency> + '_ {
        self.dependencies
            .iter()
            .flat_map(|(task, data)| TaskDependencyDataIter::Data {
                task: *task,
                output: false,
                cell_zero: false,
                data,
            })
    }

    pub fn insert(&mut self, dependency: TaskDependency) {
        let task = match dependency {
            TaskDependency::Output(task) => task,
            TaskDependency::Cell(task, _) => task,
            TaskDependency::Collectibles(task, _) => task,
        };
        match self.dependencies.entry(task) {
            Entry::Occupied(mut e) => {
                e.get_mut().insert(dependency);
            }
            Entry::Vacant(e) => {
                e.insert(match dependency {
                    TaskDependency::Output(_) => {
                        TaskDependencyData::InlineAndOutput(TaskDependencyDataInline {
                            cell_zero: None,
                        })
                    }
                    TaskDependency::Cell(_, cell_id) if cell_id.index == 0 => {
                        TaskDependencyData::Inline(TaskDependencyDataInline {
                            cell_zero: Some(cell_id.type_id),
                        })
                    }
                    TaskDependency::Cell(_, cell_id) => {
                        TaskDependencyData::Boxed(TaskDependencyDataBoxed {
                            dependencies: Box::new(AutoSet::from([DependencyType::Cell(cell_id)])),
                        })
                    }
                    TaskDependency::Collectibles(_, trait_id) => {
                        TaskDependencyData::Boxed(TaskDependencyDataBoxed {
                            dependencies: Box::new(AutoSet::from([DependencyType::Collectibles(
                                trait_id,
                            )])),
                        })
                    }
                });
            }
        }
    }
}

impl IntoIterator for TaskDependencies {
    type Item = TaskDependency;
    type IntoIter = impl Iterator<Item = Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.dependencies
            .into_iter()
            .flat_map(|(task, data)| TaskDependencyDataIntoIter::Data { task, data })
    }
}
