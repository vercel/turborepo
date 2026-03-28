use std::sync::Arc;

use turborepo_query_api::QueryServer;
use turborepo_ui::wui::{event::WebUIEvent, query::SharedState};

use crate::run::Run;

pub async fn start_web_ui_server(
    rx: tokio::sync::mpsc::UnboundedReceiver<WebUIEvent>,
    run: Arc<Run>,
    query_server: Arc<dyn QueryServer>,
) -> Result<(), turborepo_ui::Error> {
    let state = SharedState::default();
    let subscriber = turborepo_ui::wui::subscriber::Subscriber::new(rx);
    tokio::spawn(subscriber.watch(state.clone()));

    let run: Arc<dyn turborepo_query_api::QueryRun> = run;
    query_server
        .run_web_ui_server(state, run)
        .await
        .map_err(|e| {
            let wui_err = turborepo_ui::wui::Error::Server(std::io::Error::other(e));
            turborepo_ui::Error::Wui(wui_err)
        })?;

    Ok(())
}
