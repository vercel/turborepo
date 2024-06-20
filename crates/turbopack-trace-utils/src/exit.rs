use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::Result;
use tokio::{select, sync::mpsc, task::JoinSet};

/// A guard for the exit handler. When dropped, the exit guard will be dropped.
/// It might also be dropped on Ctrl-C.
pub struct ExitGuard<T>(Arc<Mutex<Option<T>>>);

impl<T> Drop for ExitGuard<T> {
    fn drop(&mut self) {
        drop(self.0.lock().unwrap().take())
    }
}

impl<T: Send + 'static> ExitGuard<T> {
    /// Drop a guard when Ctrl-C is pressed or the [ExitGuard] is dropped.
    pub fn new(guard: T) -> Result<Self> {
        let guard = Arc::new(Mutex::new(Some(guard)));
        {
            let guard = guard.clone();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.unwrap();
                drop(guard.lock().unwrap().take());
                std::process::exit(0);
            });
        }
        Ok(ExitGuard(guard))
    }
}

type BoxExitFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// The singular global ExitHandler. This is primarily used to ensure
/// `ExitHandler::listen` is only called once.
///
/// The global handler is intentionally not exposed, so that APIs that depend on
/// exit behavior are required to take the `ExitHandler`. This ensures that the
/// `ExitHandler` is configured before these APIs are run, and that these
/// consumers can be used with a callback (e.g. a mock) instead.
static GLOBAL_EXIT_HANDLER: OnceLock<Arc<ExitHandler>> = OnceLock::new();

pub struct ExitHandler {
    tx: mpsc::UnboundedSender<BoxExitFuture>,
}

impl ExitHandler {
    /// Waits for `SIGINT` using [`tokio::signal::ctrl_c`], and exits the
    /// process with exit code `0` after running any futures scheduled with
    /// [`ExitHandler::on_exit`].
    ///
    /// As this uses global process signals, this must only be called once, and
    /// will panic if called multiple times. Use this when you own the
    /// process (e.g. `turbopack-cli`).
    ///
    /// If you don't own the process (e.g. you're called as a library, such as
    /// in `next-swc`), use [`ExitHandler::new_trigger`] instead.
    ///
    /// This may listen for other signals, like `SIGTERM` or `SIGPIPE` in the
    /// future.
    pub fn listen() -> &'static Arc<ExitHandler> {
        let (handler, receiver) = Self::new_receiver();
        if GLOBAL_EXIT_HANDLER.set(handler).is_err() {
            panic!("ExitHandler::listen must only be called once");
        }
        tokio::spawn(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to set ctrl_c handler");
            receiver.run_exit_handler().await;
            std::process::exit(0);
        });
        GLOBAL_EXIT_HANDLER.get().expect("value is set")
    }

    /// Creates an [`ExitHandler`] that can be manually controlled with an
    /// [`ExitReceiver`].
    ///
    /// This does not actually exit the process or listen for any signals. If
    /// you'd like that behavior, use [`ExitHandler::listen`].
    ///
    /// Because this API has no global side-effects and can be called many times
    /// within the same process, it is possible to use it to provide a mock
    /// [`ExitHandler`] inside unit tests.
    pub fn new_receiver() -> (Arc<ExitHandler>, ExitReceiver) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Arc::new(ExitHandler { tx }), ExitReceiver { rx })
    }

    /// Register this given [`Future`] to run upon process exit.
    ///
    /// As there are many ways for a process be killed that are outside of a
    /// process's own control (e.g. `SIGKILL` or `SIGSEGV`), this API is
    /// provided on a best-effort basis.
    pub fn on_exit(&self, fut: impl Future<Output = ()> + Send + 'static) {
        // realistically, this error case can only happen with a mock
        self.tx
            .send(Box::pin(fut))
            .expect("cannot send future after process exit");
    }
}

/// Provides a way to run futures scheduled with an [`ExitHandler`].
pub struct ExitReceiver {
    rx: mpsc::UnboundedReceiver<BoxExitFuture>,
}

impl ExitReceiver {
    /// Call this when the process exits to run the futures scheduled via
    /// [`ExitHandler::on_exit`].
    ///
    /// As this is intended to be used in a library context, this does not exit
    /// the process. It is expected that the process will not exit until
    /// this async method finishes executing.
    pub async fn run_exit_handler(mut self) {
        let mut set = JoinSet::new();
        while let Ok(fut) = self.rx.try_recv() {
            set.spawn(fut);
        }
        loop {
            select! {
                Some(fut) = self.rx.recv() => {
                    set.spawn(fut);
                },
                val = set.join_next() => {
                    match val {
                        Some(Ok(())) => {}
                        Some(Err(_)) => panic!("ExitHandler future panicked!"),
                        None => return,
                    }
                },
            }
        }
    }
}
