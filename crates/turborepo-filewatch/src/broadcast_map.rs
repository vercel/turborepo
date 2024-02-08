use std::{collections::HashMap, hash::Hash};

use tokio::sync::broadcast::error::SendError;

#[derive(Clone)]
pub enum HashmapEvent<V> {
    Insert(V),
    Update(V),
    Remove,
}

pub struct UpdatingHashMap<K, V>(
    HashMap<K, V>,
    tokio::sync::broadcast::Sender<(K, HashmapEvent<V>)>,
);

impl<K, V> Default for UpdatingHashMap<K, V>
where
    K: Eq + PartialEq + Hash + Clone,
    V: Clone + PartialEq,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> UpdatingHashMap<K, V>
where
    K: Eq + PartialEq + Hash + Clone,
    V: Clone + PartialEq,
{
    pub fn new() -> Self {
        HashMap::new().into()
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<SendError<(K, HashmapEvent<V>)>> {
        if let Some(e) = self.0.get(&key) {
            if e == &value {
                None
            } else {
                self.1
                    .send((key.clone(), HashmapEvent::Update(value.clone())))
                    .err()
            }
        } else {
            self.0.insert(key.clone(), value.clone());
            self.1.send((key, HashmapEvent::Insert(value))).err()
        }
    }

    pub fn remove(&mut self, key: K) -> Option<SendError<(K, HashmapEvent<V>)>> {
        if self.0.remove(&key).is_some() {
            self.1.send((key, HashmapEvent::Remove)).err()
        } else {
            None
        }
    }

    /// replace the entire map with a new one, emitting events for changes that
    /// actually have a difference
    pub fn replace(&mut self, new: HashMap<K, V>) -> Option<SendError<(K, HashmapEvent<V>)>> {
        // check items that were removed and emit
        for (k, _) in self.0.iter() {
            if !new.contains_key(k) {
                self.1.send((k.clone(), HashmapEvent::Remove)).err()?;
            }
        }

        // check items that were inserted or changed and emit
        for (k, v) in new.iter() {
            if !self.0.contains_key(k) {
                self.1
                    .send((k.clone(), HashmapEvent::Insert(v.clone())))
                    .err();
            }
            if self.0.get(k) != Some(v) {
                self.1
                    .send((k.clone(), HashmapEvent::Update(v.clone())))
                    .err();
            }
        }

        self.0 = new;
        None
    }

    pub fn as_inner(&self) -> &HashMap<K, V> {
        &self.0
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<(K, HashmapEvent<V>)> {
        self.1.subscribe()
    }
}

impl<K, V> From<HashMap<K, V>> for UpdatingHashMap<K, V>
where
    K: Eq + PartialEq + Hash + Clone,
    V: Clone,
{
    fn from(map: HashMap<K, V>) -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(100);
        let updating_map = Self(map, tx);
        for (k, v) in updating_map.0.iter() {
            updating_map
                .1
                .send((k.clone(), HashmapEvent::Insert(v.clone())))
                .ok();
        }
        updating_map
    }
}

#[cfg(test)]
mod tests {

    use tokio::runtime::Runtime;

    use super::*;

    fn run_test<T>(test: T)
    where
        T: FnOnce() + Send + 'static,
    {
        let rt = Runtime::new().unwrap();
        rt.block_on(async { test() });
    }

    #[test]
    fn test_insert() {
        run_test(|| {
            let mut map: UpdatingHashMap<String, i32> = UpdatingHashMap::new();
            let mut receiver = map.subscribe();

            assert!(map.insert("a".to_string(), 1).is_none());

            if let Ok((key, event)) = receiver.try_recv() {
                assert_eq!(key, "a");
                match event {
                    HashmapEvent::Insert(value) => assert_eq!(value, 1),
                    _ => panic!("Expected Insert event"),
                }
            } else {
                panic!("Expected an event");
            }
        });
    }

    #[test]
    fn test_update() {
        run_test(|| {
            let mut map: UpdatingHashMap<String, i32> = UpdatingHashMap::new();
            let _ = map.insert("a".to_string(), 1);
            let mut receiver = map.subscribe();

            assert!(map.insert("a".to_string(), 2).is_none());

            if let Ok((key, event)) = receiver.try_recv() {
                assert_eq!(key, "a");
                match event {
                    HashmapEvent::Update(value) => assert_eq!(value, 2),
                    _ => panic!("Expected Update event"),
                }
            } else {
                panic!("Expected an event");
            }
        });
    }

    #[test]
    fn test_remove() {
        run_test(|| {
            let mut map: UpdatingHashMap<String, i32> = UpdatingHashMap::new();
            let _ = map.insert("a".to_string(), 1);
            let mut receiver = map.subscribe();

            assert!(map.remove("a".to_string()).is_none());

            if let Ok((key, event)) = receiver.try_recv() {
                assert_eq!(key, "a");
                match event {
                    HashmapEvent::Remove => (),
                    _ => panic!("Expected Remove event"),
                }
            } else {
                panic!("Expected an event");
            }
        });
    }
}
