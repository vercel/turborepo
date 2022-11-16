use std::{
    fmt::{Debug, Display},
    sync::Arc,
    time::Duration,
};

use anyhow::Error;

pub use super::{id_factory::IdFactory, no_move_vec::NoMoveVec, once_map::*};

/// A error struct that is backed by an Arc to allow cloning errors
#[derive(Debug, Clone)]
pub struct SharedError {
    inner: Arc<Error>,
}

impl SharedError {
    pub fn new(err: Error) -> Self {
        match err.downcast::<SharedError>() {
            Ok(shared) => shared,
            Err(plain) => Self {
                inner: Arc::new(plain),
            },
        }
    }
}

impl std::error::Error for SharedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.inner.source()
    }

    fn provide<'a>(&'a self, req: &mut std::any::Demand<'a>) {
        self.inner.provide(req);
    }
}

impl Display for SharedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&*self.inner, f)
    }
}

impl From<Error> for SharedError {
    fn from(e: Error) -> Self {
        Self::new(e)
    }
}

pub struct FormatDuration<T: Copy + Into<Duration>>(pub T);

impl<T: Copy + Into<Duration>> Display for FormatDuration<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = self.0.into();
        let s = d.as_secs();
        if s > 10 {
            return write!(f, "{}s", s);
        }
        let ms = d.as_millis();
        if ms > 10 {
            return write!(f, "{}ms", ms);
        }
        write!(f, "{}ms", (d.as_micros() as f32) / 1000.0)
    }
}

impl<T: Copy + Into<Duration>> Debug for FormatDuration<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = self.0.into();
        let s = d.as_secs();
        if s > 100 {
            return write!(f, "{}s", s);
        }
        let ms = d.as_millis();
        if ms > 10000 {
            return write!(f, "{:.2}s", (ms as f32) / 1000.0);
        }
        if ms > 100 {
            return write!(f, "{}ms", ms);
        }
        write!(f, "{}ms", (d.as_micros() as f32) / 1000.0)
    }
}
