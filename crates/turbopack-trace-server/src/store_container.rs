use std::{
    ops::{Deref, DerefMut},
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::store::Store;

pub struct StoreContainer {
    store: RwLock<StoreWithGeneration>,
}

struct StoreWithGeneration {
    store: Store,
    generation: usize,
}

impl StoreContainer {
    pub fn new() -> Self {
        Self {
            store: RwLock::new(StoreWithGeneration {
                store: Store::new(),
                generation: 0,
            }),
        }
    }

    pub fn read(&self) -> StoreReadGuard<'_> {
        StoreReadGuard {
            guard: self.store.read().unwrap(),
        }
    }

    pub fn write(&self) -> StoreWriteGuard<'_> {
        let mut guard = self.store.write().unwrap();
        guard.generation += 1;
        StoreWriteGuard { guard }
    }
}

pub struct StoreReadGuard<'a> {
    guard: RwLockReadGuard<'a, StoreWithGeneration>,
}

impl<'a> StoreReadGuard<'a> {
    pub fn generation(&self) -> usize {
        self.guard.generation
    }
}

impl<'a> Deref for StoreReadGuard<'a> {
    type Target = Store;

    fn deref(&self) -> &Self::Target {
        &self.guard.store
    }
}

pub struct StoreWriteGuard<'a> {
    guard: RwLockWriteGuard<'a, StoreWithGeneration>,
}

impl<'a> StoreWriteGuard<'a> {
    pub fn generation(&self) -> usize {
        self.guard.generation
    }
}

impl<'a> Deref for StoreWriteGuard<'a> {
    type Target = Store;

    fn deref(&self) -> &Self::Target {
        &self.guard.store
    }
}

impl<'a> DerefMut for StoreWriteGuard<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard.store
    }
}
