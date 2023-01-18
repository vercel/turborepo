use std::collections::HashSet;

use anyhow::Result;
use indexmap::IndexMap;
use reqwest::{Client, Url};
use serde::Deserialize;
use turbo_tasks::{primitives::StringVc, Value};
use turbopack_core::introspect::{Introspectable, IntrospectableChildrenVc, IntrospectableVc};
use turbopack_dev_server::source::{
    headers::HeaderValue, ContentSource, ContentSourceContent, ContentSourceData,
    ContentSourceDataFilter, ContentSourceDataVary, ContentSourceResultVc, ContentSourceVc,
    NeededData, ProxyResult,
};

#[derive(Deserialize)]
struct RoutingResult {
    url: String,
    res_headers: IndexMap<String, String>,
}

#[turbo_tasks::value(shared)]
pub struct NextRouterContentSource {
    /// A wrapped content source from which we will fetch assets.
    inner: ContentSourceVc,
    address: String,
}

#[turbo_tasks::value_impl]
impl NextRouterContentSourceVc {
    #[turbo_tasks::function]
    pub fn new(inner: ContentSourceVc, address: &str) -> NextRouterContentSourceVc {
        NextRouterContentSource {
            inner,
            address: address.to_string(),
        }
        .cell()
    }
}

#[turbo_tasks::function]
fn need_data(source: ContentSourceVc, path: &str) -> ContentSourceResultVc {
    ContentSourceResultVc::need_data(
        NeededData {
            source,
            path: path.to_string(),
            vary: ContentSourceDataVary {
                url: true,
                method: true,
                headers: Some(ContentSourceDataFilter::All),
                ..Default::default()
            },
        }
        .into(),
    )
}

#[turbo_tasks::value_impl]
impl ContentSource for NextRouterContentSource {
    #[turbo_tasks::function]
    async fn get(
        self_vc: NextRouterContentSourceVc,
        path: &str,
        data: Value<ContentSourceData>,
    ) -> Result<ContentSourceResultVc> {
        let this = self_vc.await?;
        let Some(method) = &data.method else {
            return Ok(need_data(self_vc.into(), path))
        };
        let Some(url) = &data.url else {
            return Ok(need_data(self_vc.into(), path))
        };
        let Some(headers) = &data.headers else {
            return Ok(need_data(self_vc.into(), path))
        };

        let mut query_params = vec![("pathname", path), ("method", method)];
        if let Some((_, query)) = url.split_once('?') {
            query_params.push(("query", query));
        }
        let url = Url::parse_with_params(&this.address, &query_params)?;

        let mut req = Client::new().get(url);
        for (key, value) in headers.iter() {
            match value {
                HeaderValue::SingleString(v) => {
                    req = req.header(key, v);
                }
                HeaderValue::SingleBytes(v) => {
                    req = req.header(key, v.clone());
                }
                HeaderValue::MultiStrings(v) => {
                    for v in v {
                        req = req.header(key, v);
                    }
                }
                HeaderValue::MultiBytes(v) => {
                    for v in v {
                        req = req.header(key, v.clone());
                    }
                }
            }
        }

        let Ok(res) = req.send().await else {
            return Ok(this
                .inner
                .get(path, Value::new(ContentSourceData::default())));
        };

        if res.headers().get("x-nextjs-route-result") == Some(&("1".try_into()?)) {
            // TODO: We don't have a way to query a source and set additional
            // headers
            let bytes = res.bytes().await?;
            let result: RoutingResult = serde_json::from_slice(&bytes)?;

            return Ok(this
                .inner
                .get(&result.url, Value::new(ContentSourceData::default())));
        }

        Ok(ContentSourceResultVc::exact(
            ContentSourceContent::HttpProxy(
                ProxyResult {
                    status: res.status().as_u16(),
                    headers: res
                        .headers()
                        .iter()
                        .filter_map(|(key, value)| {
                            value
                                .to_str()
                                .ok()
                                .map(|v| [key.as_str().to_string(), v.to_string()])
                        })
                        .flatten()
                        .collect(),
                    // TODO: We need proper streaming support.
                    body: res.bytes().await?.into(),
                }
                .cell(),
            )
            .cell()
            .into(),
        ))
    }
}

#[turbo_tasks::value_impl]
impl Introspectable for NextRouterContentSource {
    #[turbo_tasks::function]
    fn ty(&self) -> StringVc {
        StringVc::cell("next router source".to_string())
    }

    #[turbo_tasks::function]
    fn details(&self) -> StringVc {
        StringVc::cell("handles routing by letting Next.js handle the routing.".to_string())
    }

    #[turbo_tasks::function]
    async fn children(&self) -> Result<IntrospectableChildrenVc> {
        let mut children = HashSet::new();
        if let Some(inner) = IntrospectableVc::resolve_from(self.inner).await? {
            children.insert((StringVc::cell("inner".to_string()), inner));
        }
        Ok(IntrospectableChildrenVc::cell(children))
    }
}
