use std::{
    cell::UnsafeCell,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

pub static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
const RESERVE: usize = 1000240; // 100 KiB
const MAX_RESERVE: usize = 2000480; // 200 KiB

struct ThreadLocalCounter {
    reserved: usize,
}

impl ThreadLocalCounter {
    fn add(&mut self, size: usize) {
        if self.reserved >= size {
            self.reserved -= size;
        } else {
            let offset = size - self.reserved + RESERVE;
            self.reserved = RESERVE;
            ALLOCATED.fetch_add(offset, Ordering::Relaxed);
        }
    }

    fn remove(&mut self, size: usize) {
        self.reserved += size;
        if self.reserved > MAX_RESERVE {
            let offset = self.reserved - RESERVE;
            self.reserved = RESERVE;
            ALLOCATED.fetch_sub(offset, Ordering::Relaxed);
        }
    }

    fn unload(&mut self) {
        if self.reserved > 0 {
            ALLOCATED.fetch_sub(self.reserved, Ordering::Relaxed);
            self.reserved = 0;
        }
    }
}

thread_local! {
  static LOCAL_COUNTER: UnsafeCell<ThreadLocalCounter> = UnsafeCell::new(ThreadLocalCounter { reserved: 0 });
}

pub fn add(size: usize) {
    LOCAL_COUNTER.with(|local| {
        let ptr = local.get();
        // SAFETY: This is a thread local.
        let mut local = unsafe { NonNull::new_unchecked(ptr) };
        unsafe { local.as_mut() }.add(size);
    })
}

pub fn remove(size: usize) {
    LOCAL_COUNTER.with(|local| {
        let ptr = local.get();
        // SAFETY: This is a thread local.
        let mut local = unsafe { NonNull::new_unchecked(ptr) };
        unsafe { local.as_mut() }.remove(size);
    })
}

pub fn thread_stop() {
    LOCAL_COUNTER.with(|local| {
        let ptr = local.get();
        // SAFETY: This is a thread local.
        let mut local = unsafe { NonNull::new_unchecked(ptr) };
        unsafe { local.as_mut() }.unload();
    })
}
