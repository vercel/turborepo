use std::{hash::BuildHasherDefault, mem::replace};

use auto_hash_map::{map::Entry, AutoMap, AutoSet};
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use turbo_tasks::{CellId, TaskId, TraitTypeId, ValueTypeId};

use crate::multi_iter::IntoIters4;

#[derive(Hash, Copy, Clone, PartialEq, Eq)]
pub enum TaskEdge {
    Output(TaskId),
    Cell(TaskId, CellId),
    Collectibles(TaskId, TraitTypeId),
    Child(TaskId),
}

impl TaskEdge {
    fn task_and_edge_entry(self) -> (TaskId, EdgeEntry) {
        match self {
            TaskEdge::Output(task) => (task, EdgeEntry::Output),
            TaskEdge::Cell(task, cell_id) => (task, EdgeEntry::Cell(cell_id)),
            TaskEdge::Collectibles(task, trait_type_id) => {
                (task, EdgeEntry::Collectibles(trait_type_id))
            }
            TaskEdge::Child(task) => (task, EdgeEntry::Child),
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
    fn into_dependency(self, task: TaskId) -> TaskEdge {
        match self {
            EdgeEntry::Output => TaskEdge::Output(task),
            EdgeEntry::Cell(cell_id) => TaskEdge::Cell(task, cell_id),
            EdgeEntry::Collectibles(trait_type_id) => TaskEdge::Collectibles(task, trait_type_id),
            EdgeEntry::Child => TaskEdge::Child(task),
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
            EdgesEntry::Output => IntoIters4::One([EdgeEntry::Output].into_iter()),
            EdgesEntry::Child => IntoIters4::One([EdgeEntry::Child].into_iter()),
            EdgesEntry::Cell0(type_id) => {
                IntoIters4::One([EdgeEntry::Cell(CellId { type_id, index: 0 })].into_iter())
            }
            EdgesEntry::ChildAndOutput => {
                IntoIters4::Two([EdgeEntry::Child, EdgeEntry::Output].into_iter())
            }
            EdgesEntry::ChildAndCell0(type_id) => IntoIters4::Two(
                [
                    EdgeEntry::Child,
                    EdgeEntry::Cell(CellId { type_id, index: 0 }),
                ]
                .into_iter(),
            ),
            EdgesEntry::OutputAndCell0(type_id) => IntoIters4::Two(
                [
                    EdgeEntry::Output,
                    EdgeEntry::Cell(CellId { type_id, index: 0 }),
                ]
                .into_iter(),
            ),
            EdgesEntry::ChildOutputAndCell0(type_id) => IntoIters4::Three(
                [
                    EdgeEntry::Child,
                    EdgeEntry::Output,
                    EdgeEntry::Cell(CellId { type_id, index: 0 }),
                ]
                .into_iter(),
            ),
            EdgesEntry::Complex(set) => IntoIters4::Four(set.into_iter()),
        }
    }

    fn iter(&self) -> impl Iterator<Item = EdgeEntry> + '_ {
        match self {
            EdgesEntry::Empty => unreachable!(),
            EdgesEntry::Output => IntoIters4::One([EdgeEntry::Output].into_iter()),
            EdgesEntry::Child => IntoIters4::One([EdgeEntry::Child].into_iter()),
            EdgesEntry::Cell0(type_id) => IntoIters4::One(
                [EdgeEntry::Cell(CellId {
                    type_id: *type_id,
                    index: 0,
                })]
                .into_iter(),
            ),
            EdgesEntry::ChildAndOutput => {
                IntoIters4::Two([EdgeEntry::Child, EdgeEntry::Output].into_iter())
            }
            EdgesEntry::ChildAndCell0(type_id) => IntoIters4::Two(
                [
                    EdgeEntry::Child,
                    EdgeEntry::Cell(CellId {
                        type_id: *type_id,
                        index: 0,
                    }),
                ]
                .into_iter(),
            ),
            EdgesEntry::OutputAndCell0(type_id) => IntoIters4::Two(
                [
                    EdgeEntry::Output,
                    EdgeEntry::Cell(CellId {
                        type_id: *type_id,
                        index: 0,
                    }),
                ]
                .into_iter(),
            ),
            EdgesEntry::ChildOutputAndCell0(type_id) => IntoIters4::Three(
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
            EdgesEntry::Complex(set) => IntoIters4::Four(set.iter().copied()),
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

    fn as_complex(&mut self) -> &mut ComplexSet {
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

    fn insert(&mut self, entry: EdgeEntry) -> bool {
        if self.has(entry) {
            return false;
        }
        match entry {
            EdgeEntry::Output => match self {
                EdgesEntry::Child => {
                    *self = EdgesEntry::ChildAndOutput;
                    return true;
                }
                EdgesEntry::Cell0(type_id) => {
                    *self = EdgesEntry::OutputAndCell0(*type_id);
                    return true;
                }
                EdgesEntry::ChildAndCell0(type_id) => {
                    *self = EdgesEntry::ChildOutputAndCell0(*type_id);
                    return true;
                }
                _ => {}
            },
            EdgeEntry::Child => match self {
                EdgesEntry::Output => {
                    *self = EdgesEntry::ChildAndOutput;
                    return true;
                }
                EdgesEntry::Cell0(type_id) => {
                    *self = EdgesEntry::ChildAndCell0(*type_id);
                    return true;
                }
                EdgesEntry::OutputAndCell0(type_id) => {
                    *self = EdgesEntry::ChildOutputAndCell0(*type_id);
                    return true;
                }
                _ => {}
            },
            EdgeEntry::Cell(type_id) => {
                let CellId { type_id, index } = type_id;
                if index == 0 {
                    match self {
                        EdgesEntry::Output => {
                            *self = EdgesEntry::OutputAndCell0(type_id);
                            return true;
                        }
                        EdgesEntry::Child => {
                            *self = EdgesEntry::ChildAndCell0(type_id);
                            return true;
                        }
                        EdgesEntry::ChildAndOutput => {
                            *self = EdgesEntry::ChildOutputAndCell0(type_id);
                            return true;
                        }
                        _ => {}
                    }
                }
            }
            EdgeEntry::Collectibles(_) => {}
        }
        self.as_complex().insert(entry)
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
pub struct TaskEdgesSet {
    edges: Box<AutoMap<TaskId, EdgesEntry, BuildHasherDefault<FxHasher>>>,
}

impl TaskEdgesSet {
    pub fn new() -> Self {
        Self {
            edges: Default::default(),
        }
    }

    pub fn insert(&mut self, edge: TaskEdge) -> bool {
        let (task, edge) = edge.task_and_edge_entry();
        match self.edges.entry(task) {
            Entry::Occupied(mut entry) => {
                let entry = entry.get_mut();
                entry.insert(edge)
            }
            Entry::Vacant(entry) => {
                entry.insert(EdgesEntry::from(edge));
                true
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

    pub fn into_list(self) -> TaskEdgesList {
        let mut edges = Vec::with_capacity(self.edges.len());
        self.edges.into_iter().for_each(|edge| edges.push(edge));
        TaskEdgesList {
            edges: edges.into_boxed_slice(),
        }
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
                !matches!(entry, EdgesEntry::Empty)
            }
            _ => true,
        });
        children
    }

    /// Removes all dependencies from the passed `dependencies` argument
    pub(crate) fn remove_all(&mut self, dependencies: &TaskEdgesSet) {
        self.edges.retain(|task, entry| {
            if let Some(other) = dependencies.edges.get(task) {
                for item in other.iter() {
                    entry.remove(item);
                }
                !matches!(entry, EdgesEntry::Empty)
            } else {
                true
            }
        });
    }

    pub(crate) fn remove(&mut self, child_id: TaskEdge) -> bool {
        let (task, edge) = child_id.task_and_edge_entry();
        let Entry::Occupied(mut entry) = self.edges.entry(task) else {
            return false;
        };
        let edge_entry = entry.get_mut();
        if edge_entry.remove(edge) {
            if matches!(edge_entry, EdgesEntry::Empty) {
                entry.remove();
            }
            true
        } else {
            false
        }
    }

    pub fn children(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.edges.iter().filter_map(|(task, entry)| match entry {
            EdgesEntry::Child => Some(*task),
            EdgesEntry::ChildAndOutput => Some(*task),
            EdgesEntry::ChildAndCell0(_) => Some(*task),
            EdgesEntry::ChildOutputAndCell0(_) => Some(*task),
            EdgesEntry::Complex(set) => {
                if set.contains(&EdgeEntry::Child) {
                    Some(*task)
                } else {
                    None
                }
            }
            _ => None,
        })
    }
}

impl IntoIterator for TaskEdgesSet {
    type Item = TaskEdge;
    type IntoIter = impl Iterator<Item = TaskEdge>;

    fn into_iter(self) -> Self::IntoIter {
        self.edges
            .into_iter()
            .flat_map(|(task, entry)| entry.into_iter().map(move |e| e.into_dependency(task)))
    }
}

#[derive(Default)]
pub struct TaskEdgesList {
    edges: Box<[(TaskId, EdgesEntry)]>,
}

impl TaskEdgesList {
    pub fn into_set(self) -> TaskEdgesSet {
        TaskEdgesSet {
            edges: Box::new(self.edges.into_vec().into_iter().collect()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    pub fn children(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.edges.iter().filter_map(|(task, entry)| match entry {
            EdgesEntry::Child => Some(*task),
            EdgesEntry::ChildAndOutput => Some(*task),
            EdgesEntry::ChildAndCell0(_) => Some(*task),
            EdgesEntry::ChildOutputAndCell0(_) => Some(*task),
            EdgesEntry::Complex(set) => {
                if set.contains(&EdgeEntry::Child) {
                    Some(*task)
                } else {
                    None
                }
            }
            _ => None,
        })
    }
}

impl IntoIterator for TaskEdgesList {
    type Item = TaskEdge;
    type IntoIter = impl Iterator<Item = TaskEdge>;

    fn into_iter(self) -> Self::IntoIter {
        self.edges
            .into_vec()
            .into_iter()
            .flat_map(|(task, entry)| entry.into_iter().map(move |e| e.into_dependency(task)))
    }
}
