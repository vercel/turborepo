use std::{
    fmt,
    fmt::{Debug, Formatter},
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Local};
use serde::Serialize;
use tokio::{sync::mpsc::Sender, task::JoinHandle};
use tracing::debug;
use turborepo_api_client::{
    spaces::{CreateSpaceRunPayload, SpaceTaskSummary},
    APIAuth, APIClient,
};
use turborepo_vercel_api::SpaceRun;

use crate::run::summary::Error;

pub struct SpacesClient {
    space_id: String,
    api_client: Arc<APIClient>,
    api_auth: Arc<APIAuth>,
    request_timeout: Duration,
    errors: Vec<Error>,
}

/// Once the client is done, we return any errors
/// and the SpaceRun struct
pub struct SpacesClientResult {
    pub errors: Vec<Error>,
    // Can be None because SpacesClient could error on join
    pub run: Option<SpaceRun>,
}

/// Handle on the space client, lets you send SpaceRequests to the worker
/// thread and eventually await on the worker thread to finish
pub struct SpacesClientHandle {
    handle: JoinHandle<Result<SpacesClientResult, Error>>,
    tx: Sender<SpaceRequest>,
}

impl Debug for SpacesClientHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // We can't print much more than that since handle/tx are both
        // opaque
        f.debug_struct("SpaceClientHandle").finish()
    }
}

impl SpacesClientHandle {
    pub async fn finish_run(&self, exit_code: u32, end_time: DateTime<Local>) -> Result<(), Error> {
        Ok(self
            .tx
            .send(SpaceRequest::FinishedRun {
                exit_code,
                end_time: end_time.timestamp_millis(),
            })
            .await?)
    }
    pub async fn close(self) -> SpacesClientResult {
        // Drop the transmitter to signal to the worker thread that
        // we're done sending requests
        drop(self.tx);

        // Wait for all of the requests to finish being processed
        match self.handle.await {
            Ok(Ok(spaces_client_result)) => spaces_client_result,
            Ok(Err(err)) => SpacesClientResult {
                errors: vec![err],
                run: None,
            },
            Err(e) => SpacesClientResult {
                errors: vec![e.into()],
                run: None,
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum SpaceRequest {
    FinishedRun { end_time: i64, exit_code: u32 },
    FinishedTask { summary: Box<SpaceTaskSummary> },
}

impl SpacesClient {
    pub fn new(
        space_id: Option<String>,
        api_client: Arc<APIClient>,
        api_auth: Option<Arc<APIAuth>>,
    ) -> Option<Self> {
        // If space_id is empty, we don't build a client
        let space_id = space_id?;
        let Some(api_auth) = api_auth else {
            eprintln!(
                "Error: experimentalSpaceId is enabled, but repo is not linked to API. Run `turbo \
                 link` or `turbo login` first"
            );
            return None;
        };

        Some(Self {
            space_id,
            api_client,
            api_auth,
            request_timeout: Duration::from_secs(10),
            errors: Vec::new(),
        })
    }

    pub fn start(
        mut self,
        create_run_payload: CreateSpaceRunPayload,
    ) -> Result<SpacesClientHandle, Error> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        let handle = tokio::spawn(async move {
            let run = match self.create_run(create_run_payload).await {
                Ok(run) => run,
                Err(e) => {
                    debug!("error creating space run: {}", e);
                    self.errors.push(e);
                    return Ok(SpacesClientResult {
                        errors: self.errors,
                        run: None,
                    });
                }
            };

            debug!("created run: {:?}", run);

            while let Some(req) = rx.recv().await {
                let resp = match req {
                    SpaceRequest::FinishedRun {
                        end_time,
                        exit_code,
                    } => self.finish_run_handler(&run, end_time, exit_code).await,
                    SpaceRequest::FinishedTask { summary } => {
                        self.finish_task_handler(*summary, &run).await
                    }
                };

                if let Err(e) = resp {
                    self.errors.push(e);
                }
            }

            Ok(SpacesClientResult {
                errors: self.errors,
                run: Some(run),
            })
        });

        Ok(SpacesClientHandle { handle, tx })
    }

    async fn create_run(&self, payload: CreateSpaceRunPayload) -> Result<SpaceRun, Error> {
        Ok(tokio::time::timeout(
            self.request_timeout,
            self.api_client
                .create_space_run(&self.space_id, &self.api_auth, payload),
        )
        .await??)
    }

    async fn finish_task_handler(
        &self,
        task_summary: SpaceTaskSummary,
        run: &SpaceRun,
    ) -> Result<(), Error> {
        Ok(tokio::time::timeout(
            self.request_timeout,
            self.api_client.create_task_summary(
                &self.space_id,
                &run.id,
                &self.api_auth,
                task_summary,
            ),
        )
        .await??)
    }

    // Called by the worker thread upon receiving a SpaceRequest::FinishedRun
    async fn finish_run_handler(
        &self,
        run: &SpaceRun,
        end_time: i64,
        exit_code: u32,
    ) -> Result<(), Error> {
        Ok(tokio::time::timeout(
            self.request_timeout,
            self.api_client.finish_space_run(
                &self.space_id,
                &run.id,
                &self.api_auth,
                end_time,
                exit_code,
            ),
        )
        .await??)
    }
}
