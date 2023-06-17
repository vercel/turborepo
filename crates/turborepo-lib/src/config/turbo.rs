use serde::{Deserialize, Serialize};

use crate::{opts::RemoteCacheOpts, task_graph::Pipeline};

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpacesJson {
    pub id: Option<String>,
    #[serde(flatten)]
    pub other: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TurboJson {
    #[serde(flatten)]
    other: serde_json::Value,
    pub(crate) remote_cache_opts: Option<RemoteCacheOpts>,
    pub space_id: Option<String>,
    #[allow(dead_code)]
    pub pipeline: Pipeline,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_spaces: Option<SpacesJson>,
}
