use std::{
    collections::{hash_map::Entry, HashMap},
    hash::BuildHasherDefault,
    mem::replace,
};

use auto_hash_map::AutoSet;
use rustc_hash::FxHasher;
use turbo_tasks::{CellId, TaskId, TraitTypeId, ValueTypeId};

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

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            IntoIters::One(iter) => iter.size_hint(),
            IntoIters::Two(iter) => iter.size_hint(),
            IntoIters::Three(iter) => iter.size_hint(),
            IntoIters::Four(iter) => iter.size_hint(),
        }
    }
}

#[derive(Hash, Copy, Clone, PartialEq, Eq)]
pub enum TaskDependency {
    Output(TaskId),
    Cell(TaskId, CellId),
    Collectibles(TaskId, TraitTypeId),
    Child(TaskId),
}

impl TaskDependency {
    fn task_and_edge_entry(self) -> (TaskId, EdgeEntry) {
        match self {
            TaskDependency::Output(task) => (task, EdgeEntry::Output),
            TaskDependency::Cell(task, cell_id) => (task, EdgeEntry::Cell(cell_id)),
            TaskDependency::Collectibles(task, trait_type_id) => {
                (task, EdgeEntry::Collectibles(trait_type_id))
            }
            TaskDependency::Child(task) => (task, EdgeEntry::Child),
        }
    }
}

#[derive(Hash, Copy, Clone, PartialEq, Eq)]
enum EdgeEntry {
    Output,
    Child,
    Cell(CellId),
    Collectibles(TraitTypeId),
}

impl EdgeEntry {
    fn into_dependency(self, task: TaskId) -> TaskDependency {
        match self {
            EdgeEntry::Output => TaskDependency::Output(task),
            EdgeEntry::Cell(cell_id) => TaskDependency::Cell(task, cell_id),
            EdgeEntry::Collectibles(trait_type_id) => {
                TaskDependency::Collectibles(task, trait_type_id)
            }
            EdgeEntry::Child => TaskDependency::Child(task),
        }
    }
}

type ComplexSet = AutoSet<EdgeEntry, BuildHasherDefault<FxHasher>, 3>;

enum EdgesEntry {
    Empty,
    Output,
    Child,
    ChildAndOutput,
    Cell0(ValueTypeId),
    ChildAndCell0(ValueTypeId),
    OutputAndCell0(ValueTypeId),
    ChildOutputAndCell0(ValueTypeId),
    Complex(Box<ComplexSet>),
}

impl EdgesEntry {
    fn from(entry: EdgeEntry) -> Self {
        match entry {
            EdgeEntry::Output => EdgesEntry::Output,
            EdgeEntry::Child => EdgesEntry::Child,
            EdgeEntry::Cell(CellId { type_id, index }) => {
                if index == 0 {
                    EdgesEntry::Cell0(type_id)
                } else {
                    let mut set = AutoSet::default();
                    set.insert(EdgeEntry::Cell(CellId { type_id, index }));
                    EdgesEntry::Complex(Box::new(set))
                }
            }
            EdgeEntry::Collectibles(trait_type_id) => {
                let mut set = AutoSet::default();
                set.insert(EdgeEntry::Collectibles(trait_type_id));
                EdgesEntry::Complex(Box::new(set))
            }
        }
    }

    fn len(&self) -> usize {
        match self {
            EdgesEntry::Empty => unreachable!(),
            EdgesEntry::Output => 1,
            EdgesEntry::Child => 1,
            EdgesEntry::Cell0(_) => 1,
            EdgesEntry::OutputAndCell0(_) => 2,
            EdgesEntry::ChildAndCell0(_) => 2,
            EdgesEntry::ChildAndOutput => 2,
            EdgesEntry::ChildOutputAndCell0(_) => 3,
            EdgesEntry::Complex(set) => set.len(),
        }
    }

    fn into_iter(self) -> impl Iterator<Item = EdgeEntry> {
        match self {
            EdgesEntry::Empty => unreachable!(),
            EdgesEntry::Output => IntoIters::One([EdgeEntry::Output].into_iter()),
            EdgesEntry::Child => IntoIters::One([EdgeEntry::Child].into_iter()),
            EdgesEntry::Cell0(type_id) => {
                IntoIters::One([EdgeEntry::Cell(CellId { type_id, index: 0 })].into_iter())
            }
            EdgesEntry::ChildAndOutput => {
                IntoIters::Two([EdgeEntry::Child, EdgeEntry::Output].into_iter())
            }
            EdgesEntry::ChildAndCell0(type_id) => IntoIters::Two(
                [
                    EdgeEntry::Child,
                    EdgeEntry::Cell(CellId { type_id, index: 0 }),
                ]
                .into_iter(),
            ),
            EdgesEntry::OutputAndCell0(type_id) => IntoIters::Two(
                [
                    EdgeEntry::Output,
                    EdgeEntry::Cell(CellId { type_id, index: 0 }),
                ]
                .into_iter(),
            ),
            EdgesEntry::ChildOutputAndCell0(type_id) => IntoIters::Three(
                [
                    EdgeEntry::Child,
                    EdgeEntry::Output,
                    EdgeEntry::Cell(CellId { type_id, index: 0 }),
                ]
                .into_iter(),
            ),
            EdgesEntry::Complex(set) => IntoIters::Four(set.into_iter()),
        }
    }

    fn iter(&self) -> impl Iterator<Item = EdgeEntry> + '_ {
        match self {
            EdgesEntry::Empty => unreachable!(),
            EdgesEntry::Output => IntoIters::One([EdgeEntry::Output].into_iter()),
            EdgesEntry::Child => IntoIters::One([EdgeEntry::Child].into_iter()),
            EdgesEntry::Cell0(type_id) => IntoIters::One(
                [EdgeEntry::Cell(CellId {
                    type_id: *type_id,
                    index: 0,
                })]
                .into_iter(),
            ),
            EdgesEntry::ChildAndOutput => {
                IntoIters::Two([EdgeEntry::Child, EdgeEntry::Output].into_iter())
            }
            EdgesEntry::ChildAndCell0(type_id) => IntoIters::Two(
                [
                    EdgeEntry::Child,
                    EdgeEntry::Cell(CellId {
                        type_id: *type_id,
                        index: 0,
                    }),
                ]
                .into_iter(),
            ),
            EdgesEntry::OutputAndCell0(type_id) => IntoIters::Two(
                [
                    EdgeEntry::Output,
                    EdgeEntry::Cell(CellId {
                        type_id: *type_id,
                        index: 0,
                    }),
                ]
                .into_iter(),
            ),
            EdgesEntry::ChildOutputAndCell0(type_id) => IntoIters::Three(
                [
                    EdgeEntry::Child,
                    EdgeEntry::Output,
                    EdgeEntry::Cell(CellId {
                        type_id: *type_id,
                        index: 0,
                    }),
                ]
                .into_iter(),
            ),
            EdgesEntry::Complex(set) => IntoIters::Four(set.iter().copied()),
        }
    }

    fn has(&self, entry: EdgeEntry) -> bool {
        match entry {
            EdgeEntry::Output => {
                if let EdgesEntry::Complex(set) = self {
                    set.contains(&EdgeEntry::Output)
                } else {
                    matches!(
                        self,
                        EdgesEntry::Output
                            | EdgesEntry::OutputAndCell0(_)
                            | EdgesEntry::ChildAndOutput
                            | EdgesEntry::ChildOutputAndCell0(_)
                    )
                }
            }
            EdgeEntry::Child => {
                if let EdgesEntry::Complex(set) = self {
                    set.contains(&EdgeEntry::Child)
                } else {
                    matches!(
                        self,
                        EdgesEntry::Child
                            | EdgesEntry::ChildAndOutput
                            | EdgesEntry::ChildAndCell0(_)
                            | EdgesEntry::ChildOutputAndCell0(_)
                    )
                }
            }
            EdgeEntry::Cell(cell_id) => {
                if let EdgesEntry::Complex(set) = self {
                    set.contains(&EdgeEntry::Cell(cell_id))
                } else if cell_id.index == 0 {
                    match self {
                        EdgesEntry::Cell0(type_id) => *type_id == cell_id.type_id,
                        EdgesEntry::OutputAndCell0(type_id) => *type_id == cell_id.type_id,
                        EdgesEntry::ChildAndCell0(type_id) => *type_id == cell_id.type_id,
                        EdgesEntry::ChildOutputAndCell0(type_id) => *type_id == cell_id.type_id,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            EdgeEntry::Collectibles(trait_type_id) => {
                if let EdgesEntry::Complex(set) = self {
                    set.contains(&EdgeEntry::Collectibles(trait_type_id))
                } else {
                    false
                }
            }
        }
    }

    fn into_complex(&mut self) -> &mut ComplexSet {
        match self {
            EdgesEntry::Complex(set) => set,
            _ => {
                let items = replace(self, EdgesEntry::Output).into_iter().collect();
                *self = EdgesEntry::Complex(Box::new(items));
                let EdgesEntry::Complex(set) = self else {
                    unreachable!();
                };
                set
            }
        }
    }

    fn insert(&mut self, entry: EdgeEntry) {
        if self.has(entry) {
            return;
        }
        match entry {
            EdgeEntry::Output => match self {
                EdgesEntry::Child => {
                    *self = EdgesEntry::ChildAndOutput;
                    return;
                }
                EdgesEntry::Cell0(type_id) => {
                    *self = EdgesEntry::OutputAndCell0(*type_id);
                    return;
                }
                _ => {}
            },
            EdgeEntry::Child => match self {
                EdgesEntry::Output => {
                    *self = EdgesEntry::ChildAndOutput;
                    return;
                }
                EdgesEntry::Cell0(type_id) => {
                    *self = EdgesEntry::ChildAndCell0(*type_id);
                    return;
                }
                _ => {}
            },
            EdgeEntry::Cell(type_id) => {
                let CellId { type_id, index } = type_id;
                if index == 0 {
                    match self {
                        EdgesEntry::Output => {
                            *self = EdgesEntry::OutputAndCell0(type_id);
                            return;
                        }
                        EdgesEntry::Child => {
                            *self = EdgesEntry::ChildAndCell0(type_id);
                            return;
                        }
                        _ => {}
                    }
                }
            }
            EdgeEntry::Collectibles(_) => {}
        }
        self.into_complex().insert(entry);
    }

    fn remove(&mut self, entry: EdgeEntry) {
        if !self.has(entry) {
            return;
        }
        match entry {
            EdgeEntry::Output => match self {
                EdgesEntry::Output => {
                    *self = EdgesEntry::Empty;
                    return;
                }
                EdgesEntry::ChildAndOutput => {
                    *self = EdgesEntry::Child;
                    return;
                }
                EdgesEntry::OutputAndCell0(type_id) => {
                    *self = EdgesEntry::Cell0(*type_id);
                    return;
                }
                _ => {}
            },
            EdgeEntry::Child => match self {
                EdgesEntry::Child => {
                    *self = EdgesEntry::Empty;
                    return;
                }
                EdgesEntry::ChildAndOutput => {
                    *self = EdgesEntry::Output;
                    return;
                }
                EdgesEntry::ChildAndCell0(type_id) => {
                    *self = EdgesEntry::Cell0(*type_id);
                    return;
                }
                _ => {}
            },
            EdgeEntry::Cell(type_id) => {
                let CellId { type_id, index } = type_id;
                if index == 0 {
                    match self {
                        EdgesEntry::Cell0(_) => {
                            *self = EdgesEntry::Empty;
                            return;
                        }
                        EdgesEntry::OutputAndCell0(_) => {
                            *self = EdgesEntry::Output;
                            return;
                        }
                        EdgesEntry::ChildAndCell0(_) => {
                            *self = EdgesEntry::Child;
                            return;
                        }
                        _ => {}
                    }
                }
            }
            EdgeEntry::Collectibles(_) => {}
        }
        if let EdgesEntry::Complex(set) = self {
            set.remove(&entry);
            if set.is_empty() {
                *self = EdgesEntry::Empty;
            }
        }
    }

    fn shrink_to_fit(&mut self) {
        if let EdgesEntry::Complex(set) = self {
            set.shrink_to_fit();
        }
    }
}

#[derive(Default)]
pub struct TaskDependencySet {
    map: HashMap<TaskId, EdgesEntry, BuildHasherDefault<FxHasher>>,
}

impl TaskDependencySet {
    pub fn new() -> Self {
        Self {
            map: HashMap::default(),
        }
    }

    pub fn insert(&mut self, edge: TaskDependency) {
        let (task, edge) = edge.task_and_edge_entry();
        match self.map.entry(task) {
            Entry::Occupied(mut entry) => {
                let entry = entry.get_mut();
                entry.insert(edge);
            }
            Entry::Vacant(entry) => {
                entry.insert(EdgesEntry::from(edge));
            }
        }
    }

    pub fn shrink_to_fit(&mut self) {
        for entry in self.map.values_mut() {
            entry.shrink_to_fit();
        }
        self.map.shrink_to_fit();
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn len(&self) -> usize {
        self.map.iter().map(|(_, entry)| entry.len()).sum()
    }

    pub fn into_list(self) -> TaskDependenciesList {
        let mut edges = Vec::with_capacity(self.len());
        self.map.into_iter().for_each(|edge| edges.push(edge));
        TaskDependenciesList { edges }
    }
}

impl IntoIterator for TaskDependencySet {
    type Item = TaskDependency;
    type IntoIter = impl Iterator<Item = TaskDependency>;

    fn into_iter(self) -> Self::IntoIter {
        self.map
            .into_iter()
            .flat_map(|(task, entry)| entry.into_iter().map(move |e| e.into_dependency(task)))
    }
}

#[derive(Default)]
pub struct TaskDependenciesList {
    edges: Vec<(TaskId, EdgesEntry)>,
}

impl TaskDependenciesList {
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    pub(crate) fn remove(&mut self, dependencies: &TaskDependencySet) {
        self.edges.retain_mut(|(task, entry)| {
            if let Some(other) = dependencies.map.get(task) {
                for item in other.iter() {
                    entry.remove(item)
                }
                if matches!(entry, EdgesEntry::Empty) {
                    false
                } else {
                    true
                }
            } else {
                true
            }
        });
    }
}

impl IntoIterator for TaskDependenciesList {
    type Item = TaskDependency;
    type IntoIter = impl Iterator<Item = TaskDependency>;

    fn into_iter(self) -> Self::IntoIter {
        self.edges
            .into_iter()
            .flat_map(|(task, entry)| entry.into_iter().map(move |e| e.into_dependency(task)))
    }
}
