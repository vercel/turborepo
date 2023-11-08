use chrono::{DateTime, Local};
use reqwest::{Method, RequestBuilder};
use serde::Serialize;
use turbopath::AnchoredSystemPath;
use turborepo_vercel_api::SpaceRun;

use crate::{retry, APIAuth, APIClient, Client, Error};

#[derive(Debug, Serialize)]
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

#[derive(Default, Debug, Serialize)]
pub struct SpacesCacheStatus {
    pub status: String,
    pub source: Option<String>,
    pub time_saved: u32,
}

#[derive(Default, Debug, Serialize)]
pub struct SpaceTaskSummary {
    pub key: String,
    pub name: String,
    pub workspace: String,
    pub hash: String,
    pub start_time: i64,
    pub end_time: i64,
    pub cache: SpacesCacheStatus,
    pub exit_code: u32,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
    pub logs: String,
}

#[derive(Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SpaceRunType {
    Turbo,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSpaceRunPayload {
    pub start_time: i64,
    pub status: RunStatus,
    #[serde(rename = "type")]
    pub ty: SpaceRunType, // Hardcoded to "TURBO"
    pub command: String,
    #[serde(rename = "repositoryPath")]
    pub package_inference_root: String,
    #[serde(rename = "context")]
    pub run_context: &'static str,
    pub git_branch: Option<String>,
    pub git_sha: Option<String>,
    #[serde(rename = "originationUser")]
    pub user: String,
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
        user: String,
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
            ty: SpaceRunType::Turbo,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishSpaceRunPayload {
    status: RunStatus,
    end_time: i64,
    exit_code: i32,
}

impl FinishSpaceRunPayload {
    pub fn new(end_time: i64, exit_code: i32) -> Self {
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
        api_auth: &APIAuth,
        payload: CreateSpaceRunPayload,
    ) -> Result<SpaceRun, Error> {
        let url = format!("/v0/spaces/{}/runs", space_id);
        let request_builder = self
            .create_request_builder(&url, api_auth, Method::POST)
            .await?
            .json(&payload);

        let response = retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    pub async fn create_task_summary(
        &self,
        space_id: &str,
        run_id: &str,
        api_auth: &APIAuth,
        task: SpaceTaskSummary,
    ) -> Result<(), Error> {
        let request_builder = self
            .create_request_builder(
                &format!("/v0/spaces/{}/runs/{}/tasks", space_id, run_id),
                api_auth,
                Method::POST,
            )
            .await?
            .json(&task);

        retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(())
    }

    pub async fn finish_space_run(
        &self,
        space_id: &str,
        run_id: &str,
        api_auth: &APIAuth,
        end_time: i64,
        exit_code: i32,
    ) -> Result<(), Error> {
        let url = format!("/v0/spaces/{}/runs/{}", space_id, run_id);

        let payload = FinishSpaceRunPayload::new(end_time, exit_code);

        let request_builder = self
            .create_request_builder(&url, api_auth, Method::PATCH)
            .await?
            .json(&payload);

        retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(())
    }
}
