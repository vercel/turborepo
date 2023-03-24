use async_stream::stream as generator;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use turbo_tasks_bytes::Stream;
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::asset::AssetVc;
use turbopack_dev_server::source::Body;

use crate::{
    pool::NodeJsOperation, route_matcher::Param, source_map::trace_stack, ResponseHeaders,
    StructuredError,
};

pub(crate) mod error_page;
pub mod issue;
pub mod node_api_source;
pub mod render_proxy;
pub mod render_static;
pub mod rendered_source;

#[turbo_tasks::value(shared)]
#[serde(rename_all = "camelCase")]
pub struct RenderData {
    params: IndexMap<String, Param>,
    method: String,
    url: String,
    raw_query: String,
    raw_headers: Vec<(String, String)>,
    path: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum RenderStaticOutgoingMessage<'a> {
    Headers { data: &'a RenderData },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum RenderProxyOutgoingMessage<'a> {
    Headers { data: &'a RenderData },
    BodyChunk { data: &'a [u8] },
    BodyEnd,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum RenderProxyIncomingMessage {
    Headers { data: ResponseHeaders },
    Error(StructuredError),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum RenderStaticIncomingMessage {
    #[serde(rename_all = "camelCase")]
    Response {
        status_code: u16,
        headers: Vec<(String, String)>,
        body: String,
    },
    Headers {
        data: ResponseHeaders,
    },
    Rewrite {
        path: String,
    },
    Error(StructuredError),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum RenderBodyChunks {
    BodyChunk { data: Vec<u8> },
    BodyEnd,
    Error(StructuredError),
}

pub(crate) fn stream_body_chunks(
    mut operation: NodeJsOperation,
    intermediate_asset: AssetVc,
    intermediate_output_path: FileSystemPathVc,
    project_dir: FileSystemPathVc,
) -> Body {
    let chunks = Stream::new_open(
        vec![],
        Box::pin(generator! {
            macro_rules! tri {
                ($exp:expr) => {
                    match $exp {
                        Ok(v) => v,
                        Err(e) => {
                            operation.disallow_reuse();
                            yield Err(e.into());
                            return;
                        }
                    }
                }
            }

            loop {
                match tri!(operation.recv().await) {
                    RenderBodyChunks::BodyChunk { data } => {
                        yield Ok(data.into());
                    }
                    RenderBodyChunks::BodyEnd => break,
                    RenderBodyChunks::Error(error) => {
                        let trace =
                            trace_stack(error, intermediate_asset, intermediate_output_path, project_dir).await;
                        let e = trace.map_or_else(Into::into, Into::into);
                        yield Err(e);
                        break;
                    }
                }
            }
        }),
    );
    Body::from_stream(chunks.read())
}
