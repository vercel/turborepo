use anyhow::Result;
use tracing::{debug, error};

use crate::{commands::CommandBase, run::Run, signal::SignalHandler};

pub async fn run(base: CommandBase) -> Result<i32> {
    // set up signal handler here and then do a select between the run and the
    // signal handler finishing how to handle registering callbacks?
    let handler = SignalHandler::new(tokio::signal::ctrl_c());
    let run_subscriber = handler
        .subscribe()
        .expect("handler shouldn't close immediately after opening");

    let mut run = Run::new(&base);
    debug!("using the experimental rust codepath");
    debug!("configured run struct: {:?}", run);
    let run_fut = run.run(run_subscriber);
    let handler_fut = handler.done();
    tokio::select! {
        biased;
        // If we get a handler exit at the same time as a run finishes we choose that
        // future to display that we're respecting user input
        _ = handler_fut => {
            // We caught a signal, which already called the close handlers
            Ok(1)
        }
        result = run_fut => {
            // we want to "unsubscribe" at this point
            // closing.close();
            // Run finished so close the signal handler
            handler.close().await;
            match result {
                Ok(code) => Ok(code),
                Err(err) => {
                    error!("run failed: {}", err);
                    Err(err)
                }
            }
        },
    }
}
