use std::hash::BuildHasherDefault;

use auto_hash_map::{map::Entry, AutoMap, AutoSet};
use rustc_hash::FxHasher;
use turbo_tasks::{CellId, TaskId, TraitTypeId, ValueTypeId};

#[derive(Hash, Copy, Clone, PartialEq, Eq)]
pub enum TaskDependency {
    Output(TaskId),
    Cell(TaskId, CellId),
    Collectibles(TaskId, TraitTypeId),
}

#[derive(Hash, Copy, Clone, PartialEq, Eq)]
enum EdgeEntry {
    Output,
    Cell(CellId),
    Collectibles(TraitTypeId),
}

type ComplexSet = AutoSet<EdgeEntry, BuildHasherDefault<FxHasher>, 3>;

enum EdgesEntry {
    Output,
    Cell0(ValueTypeId),
    OutputAndCell0(ValueTypeId),
    Complex(Box<ComplexSet>),
}

#[derive(Default)]
pub struct TaskDependencySet {
    map: AutoMap<TaskId, EdgesEntry>,
}

impl TaskDependencySet {
    pub fn new() -> Self {
        Self {
            map: AutoMap::default(),
        }
    }

    pub fn insert(&mut self, edge: TaskDependency) {
        match edge {
            TaskDependency::Output(task) => match self.map.entry(task) {
                Entry::Occupied(mut entry) => {
                    let entry = entry.get_mut();
                    match entry {
                        EdgesEntry::Output => {}
                        EdgesEntry::OutputAndCell0(_) => {}
                        EdgesEntry::Cell0(type_id) => {
                            let mut set = AutoSet::default();
                            set.insert(EdgeEntry::Output);
                            set.insert(EdgeEntry::Cell(CellId {
                                type_id: *type_id,
                                index: 0,
                            }));
                            *entry = EdgesEntry::Complex(Box::new(set));
                        }
                        EdgesEntry::Complex(set) => {
                            set.insert(EdgeEntry::Output);
                        }
                    }
                }
                Entry::Vacant(entry) => {
                    entry.insert(EdgesEntry::Output);
                }
            },
            TaskDependency::Cell(task, cell_id) => {
                let CellId { type_id, index } = cell_id;
                if index == 0 {
                    match self.map.entry(task) {
                        Entry::Occupied(mut entry) => {
                            let entry = entry.get_mut();
                            match entry {
                                EdgesEntry::Output => {
                                    *entry = EdgesEntry::OutputAndCell0(type_id);
                                }
                                EdgesEntry::OutputAndCell0(other_type_id) => {
                                    if *other_type_id != type_id {
                                        let mut set = AutoSet::default();
                                        set.insert(EdgeEntry::Output);
                                        set.insert(EdgeEntry::Cell(CellId {
                                            type_id: *other_type_id,
                                            index: 0,
                                        }));
                                        set.insert(EdgeEntry::Cell(cell_id));
                                        *entry = EdgesEntry::Complex(Box::new(set));
                                    }
                                }
                                EdgesEntry::Cell0(other_type_id) => {
                                    if *other_type_id != type_id {
                                        let mut set = AutoSet::default();
                                        set.insert(EdgeEntry::Cell(CellId {
                                            type_id: *other_type_id,
                                            index: 0,
                                        }));
                                        set.insert(EdgeEntry::Cell(cell_id));
                                        *entry = EdgesEntry::Complex(Box::new(set));
                                    }
                                }
                                EdgesEntry::Complex(set) => {
                                    set.insert(EdgeEntry::Cell(cell_id));
                                }
                            }
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(EdgesEntry::Cell0(cell_id.type_id));
                        }
                    }
                } else {
                    self.get_complex_mut(task).insert(EdgeEntry::Cell(cell_id));
                }
            }
            TaskDependency::Collectibles(task, trait_type_id) => {
                self.get_complex_mut(task)
                    .insert(EdgeEntry::Collectibles(trait_type_id));
            }
        }
    }

    pub fn shrink_to_fit(&mut self) {
        for entry in self.map.values_mut() {
            if let EdgesEntry::Complex(set) = entry {
                set.shrink_to_fit();
            }
        }
        self.map.shrink_to_fit();
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn get_complex_mut(&mut self, task: TaskId) -> &mut ComplexSet {
        match self.map.entry(task) {
            Entry::Occupied(entry) => {
                let entry = entry.into_mut();
                match entry {
                    EdgesEntry::Output => {
                        let mut set = AutoSet::default();
                        set.insert(EdgeEntry::Output);
                        *entry = EdgesEntry::Complex(Box::new(set));
                    }
                    EdgesEntry::OutputAndCell0(type_id) => {
                        let mut set = AutoSet::default();
                        set.insert(EdgeEntry::Output);
                        set.insert(EdgeEntry::Cell(CellId {
                            type_id: *type_id,
                            index: 0,
                        }));
                        *entry = EdgesEntry::Complex(Box::new(set));
                    }
                    EdgesEntry::Cell0(type_id) => {
                        let mut set = AutoSet::default();
                        set.insert(EdgeEntry::Cell(CellId {
                            type_id: *type_id,
                            index: 0,
                        }));
                        *entry = EdgesEntry::Complex(Box::new(set));
                    }
                    EdgesEntry::Complex(set) => {
                        return set;
                    }
                }
                let EdgesEntry::Complex(set) = entry else {
                    unreachable!();
                };
                set
            }
            Entry::Vacant(entry) => {
                let EdgesEntry::Complex(set) =
                    entry.insert(EdgesEntry::Complex(Box::new(AutoSet::default())))
                else {
                    unreachable!();
                };
                set
            }
        }
    }
}

enum IntoIters<A, B, C, D> {
    One(A),
    Two(B),
    Three(C),
    Four(D),
}

impl<
        I,
        A: Iterator<Item = I>,
        B: Iterator<Item = I>,
        C: Iterator<Item = I>,
        D: Iterator<Item = I>,
    > Iterator for IntoIters<A, B, C, D>
{
    type Item = I;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IntoIters::One(iter) => iter.next(),
            IntoIters::Two(iter) => iter.next(),
            IntoIters::Three(iter) => iter.next(),
            IntoIters::Four(iter) => iter.next(),
        }
    }
}

impl IntoIterator for TaskDependencySet {
    type Item = TaskDependency;
    type IntoIter = impl Iterator<Item = TaskDependency>;

    fn into_iter(self) -> Self::IntoIter {
        self.map.into_iter().flat_map(|(task, entry)| match entry {
            EdgesEntry::Complex(set) => {
                IntoIters::One(set.into_iter().map(move |entry| match entry {
                    EdgeEntry::Output => TaskDependency::Output(task),
                    EdgeEntry::Cell(cell_id) => TaskDependency::Cell(task, cell_id),
                    EdgeEntry::Collectibles(trait_type_id) => {
                        TaskDependency::Collectibles(task, trait_type_id)
                    }
                }))
            }
            EdgesEntry::Output => IntoIters::Two([TaskDependency::Output(task)].into_iter()),
            EdgesEntry::Cell0(type_id) => IntoIters::Three(
                [TaskDependency::Cell(task, CellId { type_id, index: 0 })].into_iter(),
            ),
            EdgesEntry::OutputAndCell0(type_id) => IntoIters::Four(
                [
                    TaskDependency::Output(task),
                    TaskDependency::Cell(task, CellId { type_id, index: 0 }),
                ]
                .into_iter(),
            ),
        })
    }
}
