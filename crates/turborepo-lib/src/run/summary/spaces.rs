use std::{
    fmt,
    fmt::{Debug, Formatter},
    time::Duration,
};

use chrono::{DateTime, Local};
use serde::Serialize;
use tokio::{sync::mpsc::Sender, task::JoinHandle};
use turborepo_api_client::{
    spaces::{CreateSpaceRunPayload, RunStatus, SpaceClientSummary, SpaceRun, SpaceTaskSummary},
    APIAuth, APIClient,
};

use crate::run::summary::{Error, RunSummary};

pub struct SpacesClient {
    space_id: String,
    api_client: APIClient,
    api_auth: APIAuth,
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
            .send(SpaceRequest::Completed {
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

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum SpaceRequest {
    // These are not the greatest names, but they correspond
    // to the status tags in the Go implementation
    Completed { end_time: i64, exit_code: u32 },
    Task { summary: Box<SpaceTaskSummary> },
}

impl<'a> From<&'a RunSummary<'a>> for CreateSpaceRunPayload {
    fn from(rsm: &'a RunSummary) -> Self {
        let start_time = rsm
            .inner
            .execution_summary
            .as_ref()
            .map(|execution_summary| execution_summary.start_time)
            .unwrap_or_default()
            .timestamp_millis();
        let run_context = turborepo_ci::Vendor::get_constant().unwrap_or("LOCAL");

        CreateSpaceRunPayload {
            start_time,
            status: RunStatus::Running,
            command: rsm.synthesized_command.to_string(),
            package_inference_root: rsm
                .package_inference_root
                .map(|p| p.to_string())
                .unwrap_or_default(),
            ty: "TURBO",
            run_context,
            git_branch: rsm.inner.scm.branch.to_owned(),
            git_sha: rsm.inner.scm.sha.to_owned(),
            user: rsm.inner.user.to_owned(),
            client: SpaceClientSummary {
                id: "turbo",
                name: "Turbo",
                version: rsm.inner.turbo_version.to_string(),
            },
        }
    }
}

impl SpacesClient {
    pub fn new(
        space_id: Option<String>,
        api_client: APIClient,
        api_auth: Option<APIAuth>,
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

    pub async fn start(
        mut self,
        create_run_payload: CreateSpaceRunPayload,
    ) -> Result<SpacesClientHandle, Error> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        let handle = tokio::spawn(async move {
            let run = self.create_run(create_run_payload).await?;
            while let Some(req) = rx.recv().await {
                let resp = match req {
                    SpaceRequest::Completed {
                        end_time,
                        exit_code,
                    } => self.finish_run_handler(&run, end_time, exit_code).await,
                    SpaceRequest::Task { summary } => {
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
            self.api_client.create_space_run(&self.space_id, payload),
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
            self.api_client
                .create_task_summary(&self.space_id, &run.id, task_summary),
        )
        .await??)
    }

    // Called by the worker thread upon receiving a SpaceRequest::Completed
    async fn finish_run_handler(
        &self,
        run: &SpaceRun,
        end_time: i64,
        exit_code: u32,
    ) -> Result<(), Error> {
        Ok(tokio::time::timeout(
            self.request_timeout,
            self.api_client
                .finish_space_run(&self.space_id, &run.id, end_time, exit_code),
        )
        .await??)
    }
}
