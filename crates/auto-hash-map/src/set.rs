use std::{
    collections::hash_map::DefaultHasher,
    fmt::Debug,
    hash::{BuildHasher, BuildHasherDefault, Hash},
};

use crate::AutoMap;

#[derive(Clone)]
pub struct AutoSet<K, H = BuildHasherDefault<DefaultHasher>> {
    map: AutoMap<K, (), H>,
}

impl<K, H> Default for AutoSet<K, H> {
    fn default() -> Self {
        Self {
            map: Default::default(),
        }
    }
}

impl<K: Debug, H> Debug for AutoSet<K, H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<K> AutoSet<K, BuildHasherDefault<DefaultHasher>> {
    pub fn new() -> Self {
        Self {
            map: AutoMap::new(),
        }
    }
}

impl<K, H> AutoSet<K, H> {
    pub fn with_hasher() -> Self {
        Self {
            map: AutoMap::with_hasher(),
        }
    }
}

impl<K: Hash + Eq, H: BuildHasher + Default> AutoSet<K, H> {
    pub fn insert(&mut self, key: K) -> bool {
        self.map.insert(key, ()).is_none()
    }

    pub fn remove(&mut self, key: &K) -> bool {
        self.map.remove(key).is_some()
    }

    pub fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }
}

impl<K, H> AutoSet<K, H> {
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> Iter<'_, K> {
        Iter(self.map.iter())
    }

    pub fn into_iter(self) -> IntoIter<K> {
        IntoIter(self.map.into_iter())
    }
}

impl<K, H> IntoIterator for AutoSet<K, H> {
    type Item = K;
    type IntoIter = IntoIter<K>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_iter()
    }
}

pub struct Iter<'a, K>(super::map::Iter<'a, K, ()>);

impl<'a, K> Iterator for Iter<'a, K> {
    type Item = &'a K;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, _)| k)
    }
}

pub struct IntoIter<K>(super::map::IntoIter<K, ()>);

impl<K> Iterator for IntoIter<K> {
    type Item = K;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, _)| k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_LIST_SIZE;

    #[test]
    fn test_auto_set() {
        let mut set = AutoSet::new();
        for i in 0..MAX_LIST_SIZE * 2 {
            set.insert(i);
        }
        for i in 0..MAX_LIST_SIZE * 2 {
            assert!(set.contains(&i));
        }
        assert!(!set.contains(&(MAX_LIST_SIZE * 2)));
        for i in 0..MAX_LIST_SIZE * 2 {
            assert!(!set.remove(&(MAX_LIST_SIZE * 2)));
            assert!(set.remove(&i));
        }
        assert!(!set.remove(&(MAX_LIST_SIZE * 2)));
    }
}
