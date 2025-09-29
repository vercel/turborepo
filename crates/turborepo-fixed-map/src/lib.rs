//! A specialized map data structure with fixed keys determined at construction.
//! Provides thread-safe, one-time initialization of values for predefined keys.
//! This is exclusively used so we can lazily load `turbo.json`, cache the
//! results, and return a reference to the loaded `turbo.json`.

#![deny(clippy::all)]

use std::sync::OnceLock;

/// An error indicating that the key wasn't given to the constructor.
#[derive(Debug)]
pub struct UnknownKey;

/// A FixedMap is created with every key known at the start and cannot have
/// value removed or written over.
#[derive(Debug)]
pub struct FixedMap<K, V> {
    inner: Vec<(K, OnceLock<V>)>,
}

impl<K: Ord, V> FixedMap<K, V> {
    pub fn new(keys: impl Iterator<Item = K>) -> Self {
        let mut inner = keys.map(|key| (key, OnceLock::new())).collect::<Vec<_>>();
        inner.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
        Self { inner }
    }

    /// Get a value for a key.
    ///
    /// Returns `None` if key hasn't had a value inserted yet.
    pub fn get(&self, key: &K) -> Result<Option<&V>, UnknownKey> {
        let item_index = self
            .inner
            .binary_search_by(|(k, _)| k.cmp(key))
            .map_err(|_| UnknownKey)?;
        let (_, value) = &self.inner[item_index];
        Ok(value.get())
    }

    /// Insert a value for a key.
    ///
    /// There is no guarantee that the provided value will be the one returned.
    pub fn insert(&self, key: &K, value: V) -> Result<&V, UnknownKey> {
        let item_index = self
            .inner
            .binary_search_by(|(k, _)| k.cmp(key))
            .map_err(|_| UnknownKey)?;
        let (_, value_slot) = &self.inner[item_index];
        // We do not care about if this set was successful or if another call won out.
        // The end result is that we have a value for the key.
        let _ = value_slot.set(value);
        Ok(value_slot
            .get()
            .expect("OnceLock::set will always result in a value being present in the lock"))
    }
}

impl<K: Clone, V: Clone> Clone for FixedMap<K, V> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<K: Ord, V> FromIterator<(K, Option<V>)> for FixedMap<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, Option<V>)>>(iter: T) -> Self {
        let mut inner = iter
            .into_iter()
            .map(|(key, value)| {
                let value_slot = OnceLock::new();
                if let Some(value) = value {
                    value_slot
                        .set(value)
                        .map_err(|_| ())
                        .expect("nobody else has access to this lock yet");
                }
                (key, value_slot)
            })
            .collect::<Vec<_>>();
        inner.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
        Self { inner }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_get() {
        let map: FixedMap<i32, ()> = FixedMap::new([3, 2, 1].iter().copied());
        assert_eq!(map.get(&2).unwrap(), None);
        assert!(map.get(&4).is_err());
    }

    #[test]
    fn test_set() {
        let map: FixedMap<i32, bool> = FixedMap::new([3, 2, 1].iter().copied());
        assert!(map.insert(&4, true).is_err());
        assert_eq!(map.insert(&2, true).unwrap(), &true);
        assert_eq!(map.insert(&2, false).unwrap(), &true);
    }

    #[test]
    fn test_contention() {
        let map: FixedMap<i32, bool> = FixedMap::new([3, 2, 1].iter().copied());
        let results: Vec<_> = std::thread::scope(|scope| {
            let mut handles = vec![];
            let map = &map;
            for i in 0..16 {
                let is_even = i % 2 == 0;
                handles.push(scope.spawn(move || {
                    let val = map.insert(&1, is_even).unwrap();
                    (val, *val)
                }));
            }
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        for (val_ref, val) in results {
            assert_eq!(*val_ref, val, "all values should remain the same");
        }
    }
}
