use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use auto_hash_map::{map::Entry, AutoMap};
use nohash_hasher::BuildNoHashHasher;

use super::inner_refs::ChildLocation;

struct UpperEntry {
    middle: i32,
    right: i32,
    current: Option<ChildLocation>,
}

impl UpperEntry {
    fn new() -> Self {
        Self {
            middle: 0,
            right: 0,
            current: None,
        }
    }

    fn left() -> Self {
        Self {
            middle: 0,
            right: 0,
            current: Some(ChildLocation::Left),
        }
    }

    fn is_unset(&self) -> bool {
        self.middle == 0 && self.right == 0 && self.current.is_none()
    }

    #[must_use]
    fn add_middle(&mut self) -> bool {
        self.middle += 1;
        if self.middle > 0 && self.current.is_none() {
            self.current = Some(ChildLocation::Middle);
            true
        } else {
            false
        }
    }

    #[must_use]
    fn remove_middle(&mut self) -> Option<ChildLocation> {
        self.middle -= 1;
        if self.middle <= 0 && self.right <= 0 {
            match self.current {
                Some(ChildLocation::Middle) => {
                    self.current = None;
                    Some(ChildLocation::Middle)
                }
                Some(ChildLocation::Right) => {
                    self.current = None;
                    Some(ChildLocation::Right)
                }
                Some(ChildLocation::Left) => None,
                None => None,
            }
        } else {
            None
        }
    }

    #[must_use]
    fn add_right(&mut self) -> bool {
        self.right += 1;
        if self.right > 0 && self.current.is_none() {
            self.current = Some(ChildLocation::Right);
            true
        } else {
            false
        }
    }

    #[must_use]
    fn remove_right(&mut self) -> Option<ChildLocation> {
        self.right -= 1;
        if self.middle <= 0 && self.right <= 0 {
            match self.current {
                Some(ChildLocation::Middle) => {
                    self.current = None;
                    Some(ChildLocation::Middle)
                }
                Some(ChildLocation::Right) => {
                    self.current = None;
                    Some(ChildLocation::Right)
                }
                Some(ChildLocation::Left) => None,
                None => None,
            }
        } else {
            None
        }
    }
}

struct ArcByRef<T>(Arc<T>);

impl<T> Clone for ArcByRef<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Hash for ArcByRef<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

impl<T> PartialEq for ArcByRef<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Eq for ArcByRef<T> {}

impl<T> nohash_hasher::IsEnabled for ArcByRef<T> {}

pub struct UpperMap<K> {
    map: AutoMap<ArcByRef<K>, UpperEntry, BuildNoHashHasher<ArcByRef<K>>>,
}

impl<K> UpperMap<K> {
    pub fn new() -> Self {
        Self {
            map: AutoMap::with_hasher(),
        }
    }

    fn with_entry<T>(&mut self, key: Arc<K>, f: impl FnOnce(&mut UpperEntry) -> T) -> T {
        match self.map.entry(ArcByRef(key)) {
            Entry::Occupied(mut entry) => {
                let value = entry.get_mut();
                let r = f(value);
                if value.is_unset() {
                    entry.remove();
                }
                r
            }
            Entry::Vacant(entry) => {
                let value = entry.insert(UpperEntry::new());
                f(value)
            }
        }
    }

    pub fn init_left(&mut self, key: Arc<K>) {
        let result = self.map.insert(ArcByRef(key), UpperEntry::left());
        debug_assert!(result.is_none());
    }

    #[must_use]
    pub fn add_middle(&mut self, key: Arc<K>) -> bool {
        self.with_entry(key, |value| value.add_middle())
    }

    #[must_use]
    pub fn remove_middle(&mut self, key: Arc<K>) -> Option<ChildLocation> {
        self.with_entry(key, |value| value.remove_middle())
    }

    #[must_use]
    pub fn add_right(&mut self, key: Arc<K>) -> bool {
        self.with_entry(key, |value| value.add_right())
    }

    #[must_use]
    pub fn remove_right(&mut self, key: Arc<K>) -> Option<ChildLocation> {
        self.with_entry(key, |value| value.remove_right())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Arc<K>, ChildLocation)> + '_ {
        self.map
            .iter()
            .filter_map(|(ArcByRef(key), entry)| match entry.current {
                Some(ChildLocation::Left) => Some((key, ChildLocation::Left)),
                Some(ChildLocation::Middle) => Some((key, ChildLocation::Middle)),
                Some(ChildLocation::Right) => Some((key, ChildLocation::Right)),
                None => None,
            })
    }

    pub fn keys(&self) -> impl Iterator<Item = &Arc<K>> + '_ {
        self.map
            .iter()
            .filter(|(_, entry)| entry.current.is_some())
            .map(|(ArcByRef(key), _)| key)
    }

    pub fn is_empty(&self) -> bool {
        self.map.values().all(|entry| entry.current.is_none())
    }
}
