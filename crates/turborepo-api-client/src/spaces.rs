use chrono::{DateTime, Local};
use serde::Serialize;
use turbopath::AnchoredSystemPath;
use turborepo_vercel_api::SpaceRun;

use crate::{retry, APIClient, Error};

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Completed,
}

#[derive(Serialize)]
pub struct SpaceClientSummary {
    pub id: &'static str,
    pub name: &'static str,
    pub version: String,
}

#[derive(Serialize)]
struct SpacesCacheStatus {
    status: String,
    source: Option<String>,
    time_saved: u32,
}

#[derive(Serialize)]
pub struct SpaceTaskSummary {
    key: String,
    name: String,
    workspace: String,
    hash: String,
    start_time: i64,
    end_time: i64,
    cache: SpacesCacheStatus,
    exit_code: u32,
    dependencies: Vec<String>,
    dependents: Vec<String>,
    logs: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSpaceRunPayload {
    pub start_time: i64,
    pub status: RunStatus,
    #[serde(rename = "type")]
    pub ty: &'static str, // Hardcoded to "TURBO"
    pub command: String,
    #[serde(rename = "repositoryPath")]
    pub package_inference_root: String,
    #[serde(rename = "context")]
    pub run_context: &'static str,
    pub git_branch: Option<String>,
    pub git_sha: Option<String>,
    #[serde(rename = "originationUser")]
    pub user: Option<String>,
    pub client: SpaceClientSummary,
}

impl CreateSpaceRunPayload {
    pub fn new(
        start_time: DateTime<Local>,
        synthesized_command: &str,
        package_inference_root: Option<&AnchoredSystemPath>,
        git_branch: Option<String>,
        git_sha: Option<String>,
        version: String,
        user: Option<String>,
    ) -> Self {
        let start_time = start_time.timestamp_millis();
        let vendor = turborepo_ci::Vendor::infer();
        let run_context = vendor.map(|v| v.constant).unwrap_or("LOCAL");

        CreateSpaceRunPayload {
            start_time,
            status: RunStatus::Running,
            command: synthesized_command.to_string(),
            package_inference_root: package_inference_root
                .map(|p| p.to_string())
                .unwrap_or_default(),
            ty: "TURBO",
            run_context,
            git_branch,
            git_sha,
            user,
            client: SpaceClientSummary {
                id: "turbo",
                name: "Turbo",
                version,
            },
        }
    }
}

#[derive(Serialize)]
struct FinishSpaceRunPayload {
    status: RunStatus,
    end_time: i64,
    exit_code: u32,
}

impl FinishSpaceRunPayload {
    pub fn new(end_time: i64, exit_code: u32) -> Self {
        Self {
            status: RunStatus::Completed,
            end_time,
            exit_code,
        }
    }
}

impl APIClient {
    pub async fn create_space_run(
        &self,
        space_id: &str,
        token: &str,
        payload: CreateSpaceRunPayload,
    ) -> Result<SpaceRun, Error> {
        let mut url = self.make_url(&format!("/v0/spaces/{}/runs", space_id));
        let mut allow_auth = true;

        if self.use_preflight {
            let preflight_response = self
                .do_preflight(token, &url, "POST", "Authorization, User-Agent")
                .await?;

            allow_auth = preflight_response.allow_authorization_header;
            url = preflight_response.location.to_string();
        }

        let mut request_builder = self.client.post(&url).json(&payload);

        if allow_auth {
            request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
        }

        if let Some(constant) = turborepo_ci::Vendor::get_constant() {
            request_builder = request_builder.header("x-artifact-client-ci", constant);
        }

        let response = retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    pub async fn create_task_summary(
        &self,
        space_id: &str,
        run_id: &str,
        task: SpaceTaskSummary,
    ) -> Result<(), Error> {
        let url = self.make_url(&format!("/v0/spaces/{}/runs/{}/tasks", space_id, run_id));
        let request_builder = self.client.post(url).json(&task);

        retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(())
    }

    pub async fn finish_space_run(
        &self,
        space_id: &str,
        run_id: &str,
        end_time: i64,
        exit_code: u32,
    ) -> Result<(), Error> {
        let payload = FinishSpaceRunPayload::new(end_time, exit_code);
        let url = self.make_url(&format!("/v0/spaces/{}/runs/{}", space_id, run_id));
        let request_builder = self.client.patch(url).json(&payload);

        retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(())
    }
}
