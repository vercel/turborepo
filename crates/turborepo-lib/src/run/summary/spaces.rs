use chrono::{DateTime, Local};
use serde::Serialize;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::{APIAuth, APIClient};

struct SpacesClient {
    space_id: String,
    api_client: APIClient,
    api_auth: APIAuth,
}

#[derive(Serialize)]
struct SpacesClientSummary {
    id: String,
    name: String,
    version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum RunStatus {
    Running,
    Completed,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpacesRunPayload {
    start_time: DateTime<Local>,
    end_time: DateTime<Local>,
    status: RunStatus,
    #[serde(rename = "type")]
    ty: &'static str, // Hardcoded to "TURBO"
    exit_code: u32,
    command: String,
    repository_path: AbsoluteSystemPathBuf,
    context: String,
    client: SpacesClientSummary,
    git_branch: String,
    git_sha: String,
    #[serde(rename = "originationUser")]
    user: String,
}

impl SpacesClient {
    pub fn new(
        space_id: Option<String>,
        api_client: APIClient,
        api_auth: Option<APIAuth>,
    ) -> Option<Self> {
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
        })
    }

    async fn create_run(_payload: SpacesRunPayload) {
        todo!()
    }
}
