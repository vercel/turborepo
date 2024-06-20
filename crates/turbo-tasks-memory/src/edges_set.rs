use std::{
    collections::{hash_map::Entry, HashMap},
    hash::BuildHasherDefault,
    mem::replace,
};

use auto_hash_map::AutoSet;
use rustc_hash::FxHasher;
use smallvec::SmallVec;
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
                EdgesEntry::ChildAndCell0(type_id) => {
                    *self = EdgesEntry::ChildOutputAndCell0(*type_id);
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
                EdgesEntry::OutputAndCell0(type_id) => {
                    *self = EdgesEntry::ChildOutputAndCell0(*type_id);
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
                        EdgesEntry::ChildAndOutput => {
                            *self = EdgesEntry::ChildOutputAndCell0(type_id);
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

    fn remove(&mut self, entry: EdgeEntry) -> bool {
        if !self.has(entry) {
            return false;
        }
        // We verified that the entry is present, so any non-complex case is easier to
        // handle
        match entry {
            EdgeEntry::Output => match self {
                EdgesEntry::Output => {
                    *self = EdgesEntry::Empty;
                    return true;
                }
                EdgesEntry::ChildAndOutput => {
                    *self = EdgesEntry::Child;
                    return true;
                }
                EdgesEntry::OutputAndCell0(type_id) => {
                    *self = EdgesEntry::Cell0(*type_id);
                    return true;
                }
                _ => {}
            },
            EdgeEntry::Child => match self {
                EdgesEntry::Child => {
                    *self = EdgesEntry::Empty;
                    return true;
                }
                EdgesEntry::ChildAndOutput => {
                    *self = EdgesEntry::Output;
                    return true;
                }
                EdgesEntry::ChildAndCell0(type_id) => {
                    *self = EdgesEntry::Cell0(*type_id);
                    return true;
                }
                _ => {}
            },
            EdgeEntry::Cell(_) => match self {
                EdgesEntry::Cell0(_) => {
                    *self = EdgesEntry::Empty;
                    return true;
                }
                EdgesEntry::OutputAndCell0(_) => {
                    *self = EdgesEntry::Output;
                    return true;
                }
                EdgesEntry::ChildAndCell0(_) => {
                    *self = EdgesEntry::Child;
                    return true;
                }
                _ => {}
            },
            EdgeEntry::Collectibles(_) => {}
        }
        if let EdgesEntry::Complex(set) = self {
            if set.remove(&entry) {
                self.simplify();
                return true;
            }
        }
        false
    }

    fn shrink_to_fit(&mut self) {
        if let EdgesEntry::Complex(set) = self {
            set.shrink_to_fit();
        }
    }

    fn simplify(&mut self) {
        if let EdgesEntry::Complex(set) = self {
            match set.len() {
                0 => {
                    *self = EdgesEntry::Empty;
                }
                1 => {
                    let entry = set.iter().next().unwrap();
                    if matches!(
                        entry,
                        EdgeEntry::Output
                            | EdgeEntry::Child
                            | EdgeEntry::Cell(CellId { index: 0, .. })
                    ) {
                        *self = EdgesEntry::from(*entry);
                    }
                }
                _ => (),
            }
        }
    }
}

#[derive(Default)]
pub struct TaskDependencySet {
    edges: HashMap<TaskId, EdgesEntry, BuildHasherDefault<FxHasher>>,
}

impl TaskDependencySet {
    pub fn new() -> Self {
        Self {
            edges: HashMap::default(),
        }
    }

    pub fn insert(&mut self, edge: TaskDependency) {
        let (task, edge) = edge.task_and_edge_entry();
        match self.edges.entry(task) {
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
        for entry in self.edges.values_mut() {
            entry.shrink_to_fit();
        }
        self.edges.shrink_to_fit();
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    pub fn len(&self) -> usize {
        self.edges.iter().map(|(_, entry)| entry.len()).sum()
    }

    pub fn into_list(self) -> TaskDependenciesList {
        let mut edges = Vec::with_capacity(self.len());
        self.edges.into_iter().for_each(|edge| edges.push(edge));
        TaskDependenciesList { edges }
    }

    pub(crate) fn drain_children(&mut self) -> SmallVec<[TaskId; 64]> {
        let mut children = SmallVec::new();
        self.edges.retain(|&task, entry| match entry {
            EdgesEntry::Child => {
                children.push(task);
                false
            }
            EdgesEntry::ChildAndOutput => {
                children.push(task);
                *entry = EdgesEntry::Output;
                true
            }
            EdgesEntry::ChildAndCell0(type_id) => {
                children.push(task);
                *entry = EdgesEntry::Cell0(*type_id);
                true
            }
            EdgesEntry::ChildOutputAndCell0(type_id) => {
                children.push(task);
                *entry = EdgesEntry::OutputAndCell0(*type_id);
                true
            }
            EdgesEntry::Complex(set) => {
                if set.remove(&EdgeEntry::Child) {
                    children.push(task);
                }
                entry.simplify();
                if matches!(entry, EdgesEntry::Empty) {
                    false
                } else {
                    true
                }
            }
            _ => true,
        });
        children
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = TaskDependency> + '_ {
        self.edges
            .iter()
            .flat_map(|(task, entry)| entry.iter().map(move |e| e.into_dependency(*task)))
    }

    /// Removes all dependencies from the passed `dependencies` argument
    pub(crate) fn remove_all(&mut self, dependencies: &TaskDependencySet) {
        self.edges.retain(|task, entry| {
            if let Some(other) = dependencies.edges.get(task) {
                for item in other.iter() {
                    entry.remove(item);
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

    pub(crate) fn remove(&mut self, child_id: TaskDependency) -> bool {
        let (task, edge) = child_id.task_and_edge_entry();
        let Entry::Occupied(mut entry) = self.edges.entry(task) else {
            return false;
        };
        let entry = entry.get_mut();
        entry.remove(edge)
    }
}

impl IntoIterator for TaskDependencySet {
    type Item = TaskDependency;
    type IntoIter = impl Iterator<Item = TaskDependency>;

    fn into_iter(self) -> Self::IntoIter {
        self.edges
            .into_iter()
            .flat_map(|(task, entry)| entry.into_iter().map(move |e| e.into_dependency(task)))
    }
}

#[derive(Default)]
pub struct TaskDependenciesList {
    edges: Vec<(TaskId, EdgesEntry)>,
}

impl TaskDependenciesList {
    pub fn into_set(self) -> TaskDependencySet {
        TaskDependencySet {
            edges: self.edges.into_iter().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
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
