use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    fmt::Debug,
    hash::{BuildHasher, BuildHasherDefault, Hash},
};

use crate::{MAX_LIST_SIZE, MIN_HASH_SIZE};

#[derive(Clone)]
pub enum AutoMap<K, V, H = BuildHasherDefault<DefaultHasher>> {
    List(Vec<(K, V)>),
    Map(Box<HashMap<K, V, H>>),
}

impl<K, V, H> Default for AutoMap<K, V, H> {
    fn default() -> Self {
        Self::List(Default::default())
    }
}

impl<K: Debug, V: Debug, H> Debug for AutoMap<K, V, H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K, V> AutoMap<K, V, BuildHasherDefault<DefaultHasher>> {
    pub fn new() -> Self {
        AutoMap::List(Vec::new())
    }
}

impl<K: Eq + Hash, V, H: BuildHasher + Default> AutoMap<K, V, H> {
    fn convert_to_map(&mut self) -> &mut HashMap<K, V, H> {
        if let AutoMap::List(list) = self {
            let mut map = HashMap::with_capacity_and_hasher(MAX_LIST_SIZE * 2, Default::default());
            for (k, v) in list.drain(..) {
                map.insert(k, v);
            }
            *self = AutoMap::Map(Box::new(map));
        }
        if let AutoMap::Map(map) = self {
            map
        } else {
            unreachable!()
        }
    }

    fn convert_to_list(&mut self) -> &mut Vec<(K, V)> {
        if let AutoMap::Map(map) = self {
            let mut list = Vec::with_capacity(MAX_LIST_SIZE);
            for (k, v) in map.drain() {
                list.push((k, v));
            }
            *self = AutoMap::List(list);
        }
        if let AutoMap::List(list) = self {
            list
        } else {
            unreachable!()
        }
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self {
            AutoMap::List(list) => {
                for (k, v) in list.iter_mut() {
                    if *k == key {
                        return Some(std::mem::replace(v, value));
                    }
                }
                if list.len() == MAX_LIST_SIZE {
                    let map = self.convert_to_map();
                    map.insert(key, value);
                } else {
                    list.push((key, value));
                }
                None
            }
            AutoMap::Map(map) => map.insert(key, value),
        }
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        match self {
            AutoMap::List(list) => {
                for i in 0..list.len() {
                    if list[i].0 == *key {
                        return Some(list.swap_remove(i).1);
                    }
                }
                None
            }
            AutoMap::Map(map) => {
                let result = map.remove(key);
                if result.is_some() && map.len() < MIN_HASH_SIZE {
                    self.convert_to_list();
                }
                result
            }
        }
    }

    pub fn entry(&mut self, key: K) -> Entry<'_, K, V> {
        if self.len() >= MAX_LIST_SIZE {
            self.convert_to_map();
        } else if self.len() <= MIN_HASH_SIZE {
            self.convert_to_list();
        }
        match self {
            AutoMap::List(list) => match list.iter().position(|(k, _)| *k == key) {
                Some(index) => Entry::Occupied(OccupiedEntry::List { list, index }),
                None => Entry::Vacant(VacantEntry::List { list, key }),
            },
            AutoMap::Map(map) => match map.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    Entry::Occupied(OccupiedEntry::Map(entry))
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    Entry::Vacant(VacantEntry::Map(entry))
                }
            },
        }
    }
}

impl<K: Eq + Hash, V, H: BuildHasher> AutoMap<K, V, H> {
    pub fn get(&self, key: &K) -> Option<&V> {
        match self {
            AutoMap::List(list) => list
                .iter()
                .find_map(|(k, v)| if *k == *key { Some(v) } else { None }),
            AutoMap::Map(map) => map.get(key),
        }
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        match self {
            AutoMap::List(list) => {
                list.iter_mut()
                    .find_map(|(k, v)| if *k == *key { Some(v) } else { None })
            }
            AutoMap::Map(map) => map.get_mut(key),
        }
    }

    pub fn contains_key(&self, key: &K) -> bool {
        match self {
            AutoMap::List(list) => list.iter().any(|(k, _)| *k == *key),
            AutoMap::Map(map) => map.contains_key(key),
        }
    }
}

impl<K, V, H> AutoMap<K, V, H> {
    pub fn iter(&self) -> Iter<'_, K, V> {
        match self {
            AutoMap::List(list) => Iter::List(list.iter()),
            AutoMap::Map(map) => Iter::Map(map.iter()),
        }
    }

    pub fn into_iter(self) -> IntoIter<K, V> {
        match self {
            AutoMap::List(list) => IntoIter::List(list.into_iter()),
            AutoMap::Map(map) => IntoIter::Map(map.into_iter()),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            AutoMap::List(list) => list.len(),
            AutoMap::Map(map) => map.len(),
        }
    }
}

pub enum Iter<'a, K, V> {
    List(std::slice::Iter<'a, (K, V)>),
    Map(std::collections::hash_map::Iter<'a, K, V>),
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Iter::List(iter) => iter.next().map(|(k, v)| (k, v)),
            Iter::Map(iter) => iter.next().map(|(k, v)| (k, v)),
        }
    }
}

pub enum IntoIter<K, V> {
    List(std::vec::IntoIter<(K, V)>),
    Map(std::collections::hash_map::IntoIter<K, V>),
}

impl<K, V> Iterator for IntoIter<K, V> {
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IntoIter::List(iter) => iter.next(),
            IntoIter::Map(iter) => iter.next(),
        }
    }
}

pub enum Entry<'a, K, V> {
    Occupied(OccupiedEntry<'a, K, V>),
    Vacant(VacantEntry<'a, K, V>),
}

pub enum OccupiedEntry<'a, K, V> {
    List {
        list: &'a mut Vec<(K, V)>,
        index: usize,
    },
    Map(std::collections::hash_map::OccupiedEntry<'a, K, V>),
}

impl<'a, K, V> OccupiedEntry<'a, K, V> {
    pub fn get_mut(&mut self) -> &mut V {
        match self {
            OccupiedEntry::List { list, index } => &mut list[*index].1,
            OccupiedEntry::Map(e) => e.get_mut(),
        }
    }

    pub fn remove(self) -> V {
        match self {
            OccupiedEntry::List { list, index } => list.swap_remove(index).1,
            OccupiedEntry::Map(e) => e.remove(),
        }
    }
}

pub enum VacantEntry<'a, K, V> {
    List { list: &'a mut Vec<(K, V)>, key: K },
    Map(std::collections::hash_map::VacantEntry<'a, K, V>),
}

impl<'a, K, V> VacantEntry<'a, K, V> {
    pub fn insert(self, value: V) -> &'a mut V {
        match self {
            VacantEntry::List { list, key } => {
                list.push((key, value));
                &mut list.last_mut().unwrap().1
            }
            VacantEntry::Map(entry) => entry.insert(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_map() {
        let mut map = AutoMap::new();
        for i in 0..MAX_LIST_SIZE * 2 {
            map.insert(i, i);
        }
        for i in 0..MAX_LIST_SIZE * 2 {
            assert_eq!(map.get(&i), Some(&i));
        }
        assert_eq!(map.get(&(MAX_LIST_SIZE * 2)), None);
        for i in 0..MAX_LIST_SIZE * 2 {
            assert_eq!(map.remove(&(MAX_LIST_SIZE * 2)), None);
            assert_eq!(map.remove(&i), Some(i));
        }
        assert_eq!(map.remove(&(MAX_LIST_SIZE * 2)), None);
    }
}
