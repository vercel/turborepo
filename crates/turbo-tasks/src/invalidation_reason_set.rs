use std::{borrow::Cow, fmt::Display, mem::take};

use indexmap::{map::Entry, IndexMap, IndexSet};

use crate::{
    manager::{InvalidationReason, InvalidationReasonType},
    util::StaticOrArc,
};

#[derive(PartialEq, Eq, Hash)]
enum MapKey {
    Untyped { unique_tag: usize },
    Typed { type_pointer: usize },
}

enum MapEntry {
    Untyped {
        reason: StaticOrArc<dyn InvalidationReason>,
    },
    Single {
        reason: StaticOrArc<dyn InvalidationReason>,
        data: Cow<'static, str>,
    },
    Multiple {
        reason_ty: &'static dyn InvalidationReasonType,
        merge_data: IndexSet<Cow<'static, str>>,
    },
}

#[derive(Default)]
pub struct InvalidationReasonSet {
    next_unique_tag: usize,
    map: IndexMap<MapKey, MapEntry>,
}

impl InvalidationReasonSet {
    pub(crate) fn insert(&mut self, reason: StaticOrArc<dyn InvalidationReason>) {
        if let Some((ty, data)) = reason.merge_info() {
            let key = MapKey::Typed {
                type_pointer: ty.ptr(),
            };
            match self.map.entry(key) {
                Entry::Occupied(mut entry) => match entry.get_mut() {
                    MapEntry::Single {
                        reason: _,
                        data: existing_data,
                    } => {
                        if data == *existing_data {
                            return;
                        }
                        let mut merge_data = IndexSet::new();
                        merge_data.insert(data);
                        merge_data.insert(take(existing_data));
                        *entry.get_mut() = MapEntry::Multiple {
                            reason_ty: ty,
                            merge_data,
                        };
                    }
                    MapEntry::Multiple {
                        reason_ty: _,
                        merge_data,
                    } => {
                        merge_data.insert(data);
                    }
                    MapEntry::Untyped { reason: _ } => {
                        unreachable!();
                    }
                },
                Entry::Vacant(entry) => {
                    entry.insert(MapEntry::Single { reason, data });
                }
            }
        } else {
            let key = MapKey::Untyped {
                unique_tag: self.next_unique_tag,
            };
            self.next_unique_tag += 1;
            self.map.insert(key, MapEntry::Untyped { reason });
        }
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}

impl Display for InvalidationReasonSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let descriptions = self.map.values().map(|entry| match entry {
            MapEntry::Untyped { reason } => reason.description(),
            MapEntry::Single { reason, .. } => reason.description(),
            MapEntry::Multiple {
                reason_ty,
                merge_data,
            } => reason_ty.description(merge_data),
        });
        let count = self.map.len();
        for (i, description) in descriptions.enumerate() {
            if i > 0 {
                write!(f, ", ")?;
                if i == count - 1 {
                    write!(f, "and ")?;
                }
            }
            write!(f, "{}", description)?;
        }
        Ok(())
    }
}
