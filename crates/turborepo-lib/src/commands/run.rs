use tracing::{debug, error};

use crate::{commands::CommandBase, run, run::Run, signal::SignalHandler};

pub async fn run(base: CommandBase) -> Result<i32, run::Error> {
    let handler = SignalHandler::new(tokio::signal::ctrl_c());

    let mut run = Run::new(&base);
    debug!("using the experimental rust codepath");
    debug!("configured run struct: {:?}", run);
    let run_fut = run.run(&handler);
    let handler_fut = handler.done();
    tokio::select! {
        biased;
        // If we get a handler exit at the same time as a run finishes we choose that
        // future to display that we're respecting user input
        _ = handler_fut => {
            // We caught a signal, which already notified the subscribers
            Ok(1)
        }
        result = run_fut => {
            // Run finished so close the signal handler
            handler.close().await;
            match result {
                Ok(code) => {
                    if code != 0 {
                        error!("run failed: command  exited ({code})")
                    }
                    Ok(code)
                },
                Err(err) => {
                    error!("run failed: {}", err);
                    Err(err)
                }
            }
        },
    }
}
