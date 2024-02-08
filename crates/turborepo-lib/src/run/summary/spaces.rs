use std::{
    collections::HashSet,
    fmt,
    fmt::{Debug, Formatter},
    time::Duration,
};

use chrono::{DateTime, Local};
use futures::{stream::FuturesUnordered, StreamExt};
use itertools::Itertools;
use serde::Serialize;
use tokio::{sync::mpsc::Sender, task::JoinHandle};
use tracing::debug;
use turborepo_api_client::{
    spaces::{CreateSpaceRunPayload, SpaceTaskSummary, SpacesCacheStatus},
    APIAuth, APIClient,
};
use turborepo_cache::CacheHitMetadata;
use turborepo_vercel_api::SpaceRun;

use super::execution::TaskExecutionSummary;
use crate::{
    engine::TaskNode,
    run::{summary::Error, task_id::TaskId},
};

// There's a 4.5 MB limit on serverless requests, we limit ourselves to a
// conservative 4 MB to leave .5 MB for the other information in the response.
// https://vercel.com/guides/how-to-bypass-vercel-body-size-limit-serverless-functions
const LOG_SIZE_BYTE_LIMIT: usize = 4 * 1024 * 1024;

pub struct SpacesClient {
    space_id: String,
    api_client: APIClient,
    api_auth: APIAuth,
    request_timeout: Duration,
}

/// Once the client is done, we return any errors
/// and the SpaceRun struct
#[derive(Debug)]
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

/// A spaces client with functionality limited to sending task information
/// This client should only live while processing a task
pub struct SpacesTaskClient {
    tx: Sender<SpaceRequest>,
}

/// Information required to construct a SpacesTaskSummary
pub struct SpacesTaskInformation<'a> {
    pub task_id: TaskId<'static>,
    pub execution_summary: TaskExecutionSummary,
    pub logs: Vec<u8>,
    pub hash: String,
    pub cache_status: Option<CacheHitMetadata>,
    pub dependencies: Option<HashSet<&'a TaskNode>>,
    pub dependents: Option<HashSet<&'a TaskNode>>,
}

impl Debug for SpacesClientHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // We can't print much more than that since handle/tx are both
        // opaque
        f.debug_struct("SpaceClientHandle").finish()
    }
}

impl SpacesClientHandle {
    pub fn task_client(&self) -> SpacesTaskClient {
        SpacesTaskClient {
            tx: self.tx.clone(),
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn finish_run(&self, exit_code: i32, end_time: DateTime<Local>) -> Result<(), Error> {
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

impl SpacesTaskClient {
    async fn send_task(&self, summary: SpaceTaskSummary) -> Result<(), Error> {
        self.tx
            .send(SpaceRequest::FinishedTask {
                summary: Box::new(summary),
            })
            .await?;
        Ok(())
    }

    pub async fn finish_task<'a>(&self, info: SpacesTaskInformation<'a>) -> Result<(), Error> {
        let summary = SpaceTaskSummary::from(info);
        self.send_task(summary).await
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum SpaceRequest {
    FinishedRun { end_time: i64, exit_code: i32 },
    FinishedTask { summary: Box<SpaceTaskSummary> },
}

impl SpacesClient {
    pub fn new(
        space_id: Option<String>,
        api_client: APIClient,
        api_auth: Option<APIAuth>,
    ) -> Option<Self> {
        // If space_id is empty, we don't build a client
        let space_id = space_id?;
        let is_linked = api_auth.as_ref().map_or(false, |auth| auth.is_linked());
        if !is_linked {
            eprintln!(
                "Error: experimentalSpaceId is enabled, but repo is not linked to API. Run `turbo \
                 link` or `turbo login` first"
            );
            return None;
        }
        let api_auth = api_auth.expect("presence of api auth was just checked");

        Some(Self {
            space_id,
            api_client,
            api_auth,
            request_timeout: Duration::from_secs(10),
        })
    }

    pub fn start(
        self,
        create_run_payload: CreateSpaceRunPayload,
    ) -> Result<SpacesClientHandle, Error> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        let handle = tokio::spawn(async move {
            let mut errors = Vec::new();
            let run = match self.create_run(create_run_payload).await {
                Ok(run) => run,
                Err(e) => {
                    debug!("error creating space run: {}", e);
                    errors.push(e);
                    return Ok(SpacesClientResult { errors, run: None });
                }
            };

            debug!("created run: {:?}", run);

            let mut requests = FuturesUnordered::new();
            while let Some(req) = rx.recv().await {
                let request = match req {
                    SpaceRequest::FinishedRun {
                        end_time,
                        exit_code,
                    } => self.finish_run_handler(&run, end_time, exit_code),
                    SpaceRequest::FinishedTask { summary } => {
                        self.finish_task_handler(*summary, &run)
                    }
                };
                requests.push(request)
            }

            while let Some(response) = requests.next().await {
                let response = response.expect("spaces request panicked");
                if let Err(e) = response {
                    errors.push(e);
                }
            }

            Ok(SpacesClientResult {
                errors,
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

    fn finish_task_handler(
        &self,
        task_summary: SpaceTaskSummary,
        run: &SpaceRun,
    ) -> JoinHandle<Result<(), Error>> {
        debug!("sending task: {task_summary:?}");
        let timeout = self.request_timeout;
        let api_client = self.api_client.clone();
        let space_id = self.space_id.clone();
        let run_id = run.id.clone();
        let api_auth = self.api_auth.clone();
        tokio::spawn(async move {
            Ok(tokio::time::timeout(
                timeout,
                api_client.create_task_summary(&space_id, &run_id, &api_auth, task_summary),
            )
            .await??)
        })
    }

    // Called by the worker thread upon receiving a SpaceRequest::FinishedRun
    fn finish_run_handler(
        &self,
        run: &SpaceRun,
        end_time: i64,
        exit_code: i32,
    ) -> JoinHandle<Result<(), Error>> {
        let timeout = self.request_timeout;
        let api_client = self.api_client.clone();
        let space_id = self.space_id.clone();
        let run_id = run.id.clone();
        let api_auth = self.api_auth.clone();
        tokio::spawn(async move {
            Ok(tokio::time::timeout(
                timeout,
                api_client.finish_space_run(&space_id, &run_id, &api_auth, end_time, exit_code),
            )
            .await??)
        })
    }
}

impl<'a> From<SpacesTaskInformation<'a>> for SpaceTaskSummary {
    fn from(value: SpacesTaskInformation) -> Self {
        let SpacesTaskInformation {
            task_id,
            execution_summary,
            logs,
            hash,
            cache_status,
            dependencies,
            dependents,
        } = value;
        let TaskExecutionSummary {
            start_time,
            end_time,
            exit_code,
            ..
        } = execution_summary;
        fn stringify_nodes(deps: Option<HashSet<&crate::engine::TaskNode>>) -> Vec<String> {
            deps.into_iter()
                .flatten()
                .filter_map(|node| match node {
                    crate::engine::TaskNode::Root => None,
                    crate::engine::TaskNode::Task(dependency) => Some(dependency.to_string()),
                })
                .sorted()
                .collect()
        }
        let dependencies = stringify_nodes(dependencies);
        let dependents = stringify_nodes(dependents);

        let cache = cache_status.map_or_else(
            SpacesCacheStatus::default,
            |CacheHitMetadata { source, time_saved }| SpacesCacheStatus {
                status: turborepo_api_client::spaces::CacheStatus::Hit,
                source: Some(match source {
                    turborepo_cache::CacheSource::Local => {
                        turborepo_api_client::spaces::CacheSource::Local
                    }
                    turborepo_cache::CacheSource::Remote => {
                        turborepo_api_client::spaces::CacheSource::Remote
                    }
                }),
                time_saved,
            },
        );

        let logs = trim_logs(&logs, LOG_SIZE_BYTE_LIMIT);

        SpaceTaskSummary {
            key: task_id.to_string(),
            name: task_id.task().into(),
            workspace: task_id.package().into(),
            hash,
            cache,
            start_time,
            end_time,
            exit_code,
            dependencies,
            dependents,
            logs,
        }
    }
}

fn trim_logs(logs: &[u8], limit: usize) -> String {
    // Go JSON encoding automatically did a lossy conversion for us when
    // encoding Golang strings into JSON.
    let lossy_logs = String::from_utf8_lossy(logs);
    if lossy_logs.as_bytes().len() <= limit {
        lossy_logs.into_owned()
    } else {
        // We try to trim down the logs so that it is valid utf8
        // We attempt to parse it at every byte starting from the limit until we get a
        // valid utf8 which means we aren't cutting in the middle of a cluster.
        for start_index in (lossy_logs.as_bytes().len() - limit)..lossy_logs.as_bytes().len() {
            let log_bytes = &lossy_logs.as_bytes()[start_index..];
            if let Ok(log_str) = std::str::from_utf8(log_bytes) {
                return log_str.to_string();
            }
        }
        // This case can only happen if the limit is smaller than 4
        // and we can't even store the invalid UTF8 character
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use chrono::Local;
    use pretty_assertions::assert_eq;
    use test_case::test_case;
    use turborepo_api_client::{
        spaces::{CreateSpaceRunPayload, SpaceTaskSummary},
        APIAuth, APIClient,
    };
    use turborepo_vercel_api_mock::{
        start_test_server, EXPECTED_SPACE_ID, EXPECTED_SPACE_RUN_ID, EXPECTED_TEAM_ID,
        EXPECTED_TEAM_SLUG, EXPECTED_TOKEN,
    };

    use super::trim_logs;
    use crate::run::summary::spaces::SpacesClient;

    #[test_case(vec![] ; "empty")]
    #[test_case(vec![SpaceTaskSummary::default()] ; "one task summary")]
    #[test_case(vec![SpaceTaskSummary::default(), SpaceTaskSummary::default()] ; "two task summaries")]
    #[tokio::test]
    async fn test_spaces_client(tasks: Vec<SpaceTaskSummary>) -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));

        let api_client = APIClient::new(format!("http://localhost:{}", port), 2, "", true)?;

        let api_auth = Some(APIAuth {
            token: EXPECTED_TOKEN.to_string(),
            team_id: Some(EXPECTED_TEAM_ID.to_string()),
            team_slug: Some(EXPECTED_TEAM_SLUG.to_string()),
        });

        let spaces_client =
            SpacesClient::new(Some(EXPECTED_SPACE_ID.to_string()), api_client, api_auth).unwrap();

        let start_time = Local::now();
        let spaces_client_handle = spaces_client.start(CreateSpaceRunPayload::new(
            start_time,
            "turbo run build".to_string(),
            None,
            None,
            None,
            "".to_string(),
            "rauchg".to_string(),
        ))?;

        let mut join_set = tokio::task::JoinSet::new();
        for task_summary in tasks {
            let task_client = spaces_client_handle.task_client();
            join_set.spawn(async move { task_client.send_task(task_summary).await });
        }

        while let Some(result) = join_set.join_next().await {
            result??;
        }

        spaces_client_handle.finish_run(0, Local::now()).await?;

        let spaces_client_result = spaces_client_handle.close().await;

        assert!(spaces_client_result.errors.is_empty());
        let run = spaces_client_result.run.expect("run should exist");

        assert_eq!(run.id, EXPECTED_SPACE_RUN_ID);

        handle.abort();
        Ok(())
    }

    #[test_case(b"abcdef", 4, "cdef" ; "trims from the front of the logs")]
    #[test_case(b"abcdef", 6, "abcdef" ; "doesn't trim when logs are under limit")]
    #[test_case(&[240, 159, 146, 150, b'o', b'k'], 4, "ok" ; "doesn't cut in between utf8 chars")]
    #[test_case(&[0xa0, 0xa1, b'o', b'k'], 4, "ok" ; "handles invalid utf8 chars")]
    fn test_log_trim(logs: &[u8], limit: usize, expected: &str) {
        let actual = trim_logs(logs, limit);
        assert_eq!(expected, actual);
    }
}
