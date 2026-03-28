use futures::FutureExt;
use tokio::sync::watch::{self, Ref, error::RecvError};

#[derive(Debug)]
pub struct OptionalWatch<T>(watch::Receiver<Option<T>>);

impl<T> Clone for OptionalWatch<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// A handy wrapper around types that are watched and may be None.
/// `SomeRef` is a reference type that is known to be `Some`.
impl<T> OptionalWatch<T> {
    /// Create a new `OptionalWatch` with no initial value.
    ///
    /// Keep in mind that when the sender is dropped, down stream
    /// dependencies that are currently waiting will get a RecvError.
    pub fn new() -> (watch::Sender<Option<T>>, OptionalWatch<T>) {
        let (tx, rx) = watch::channel(None);
        (tx, OptionalWatch(rx))
    }

    /// Create a new `OptionalWatch` with an initial, unchanging value.
    #[cfg(test)]
    pub fn once(init: T) -> Self {
        let (_tx, rx) = watch::channel(Some(init));
        OptionalWatch(rx)
    }

    /// Wait for the value to be available and then return it.
    ///
    /// If you receive a `RecvError`, the sender has been dropped, meaning you
    /// will not receive any more updates. For efficiencies sake, you should
    /// exit the task and drop any senders to other dependencies so that the
    /// exit can propagate up the chain.
    pub async fn get(&mut self) -> Result<SomeRef<'_, T>, RecvError> {
        let recv = self.0.wait_for(|f| f.is_some()).await?;
        Ok(SomeRef(recv))
    }

    /// Get the current value, if it is available.
    pub fn get_immediate(&mut self) -> Option<Result<SomeRef<'_, T>, RecvError>> {
        self.get().now_or_never()
    }
}

pub struct SomeRef<'a, T>(pub(crate) Ref<'a, Option<T>>);

impl<T> std::ops::Deref for SomeRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("checked")
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use futures::FutureExt;
    use tokio::sync::watch::error::RecvError;

    /// Futures have a method that allow you to fetch the value of a future
    /// if it is immediately available. This is useful for, for example,
    /// allowing consumers to poll a future and get the value if it is
    /// available, but otherwise just continue on, rather than wait.
    #[tokio::test]
    pub async fn now_or_never_works() {
        let (tx, mut rx) = super::OptionalWatch::new();

        tx.send(Some(42)).unwrap();

        assert_eq!(*rx.get().now_or_never().unwrap().unwrap(), 42);
    }

    #[tokio::test]
    pub async fn get_returns_error_when_sender_dropped() {
        let (tx, mut rx) = super::OptionalWatch::<i32>::new();

        // Drop the sender without ever sending a value
        drop(tx);

        // get() should return RecvError, not hang
        let result = rx.get().await;
        assert!(matches!(result, Err(RecvError { .. })));
    }

    #[tokio::test]
    pub async fn get_with_timeout_returns_elapsed_when_no_value() {
        let (_tx, mut rx) = super::OptionalWatch::<i32>::new();

        // The sender is alive but never sends. A timeout should fire
        // instead of hanging forever.
        let result = tokio::time::timeout(Duration::from_millis(50), rx.get()).await;
        assert!(result.is_err(), "should have timed out");
    }
}
