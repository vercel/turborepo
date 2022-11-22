use std::ops::{Deref, DerefMut};

use parking_lot::{RwLockReadGuard, RwLockWriteGuard};

pub struct ReadGuard<'a, T: 'a, U: 'a, M: 'a + Fn(&T) -> &U> {
    inner: RwLockReadGuard<'a, T>,
    map: M,
}

impl<'a, T: 'a, U: 'a, M: 'a + Fn(&T) -> &U> ReadGuard<'a, T, U, M> {
    pub fn new(guard: RwLockReadGuard<'a, T>, map: M) -> Self {
        Self { inner: guard, map }
    }
}

impl<'a, T: 'a, U: 'a, M: 'a + Fn(&T) -> &U> Deref for ReadGuard<'a, T, U, M> {
    type Target = U;

    fn deref(&self) -> &Self::Target {
        (self.map)(&self.inner)
    }
}

pub struct WriteGuard<'a, T: 'a, U: 'a, M: 'a + Fn(&T) -> &U, MM: 'a + Fn(&mut T) -> &mut U> {
    inner: RwLockWriteGuard<'a, T>,
    map: M,
    map_mut: MM,
}

impl<'a, T: 'a, U: 'a, M: 'a + Fn(&T) -> &U, MM: 'a + Fn(&mut T) -> &mut U>
    WriteGuard<'a, T, U, M, MM>
{
    pub fn new(guard: RwLockWriteGuard<'a, T>, map: M, map_mut: MM) -> Self {
        Self {
            inner: guard,
            map,
            map_mut,
        }
    }
}

impl<'a, T: 'a, U: 'a, M: 'a + Fn(&T) -> &U, MM: 'a + Fn(&mut T) -> &mut U> Deref
    for WriteGuard<'a, T, U, M, MM>
{
    type Target = U;

    fn deref(&self) -> &Self::Target {
        (self.map)(&self.inner)
    }
}

impl<'a, T: 'a, U: 'a, M: 'a + Fn(&T) -> &U, MM: 'a + Fn(&mut T) -> &mut U> DerefMut
    for WriteGuard<'a, T, U, M, MM>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        (self.map_mut)(&mut self.inner)
    }
}
