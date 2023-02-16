use anyhow::Result;
use indexmap::IndexSet;
use turbo_tasks::{debug::ValueDebug, primitives::StringVc, Value};
use turbopack_core::{
    environment::ServerAddrVc,
    introspect::{Introspectable, IntrospectableChildrenVc, IntrospectableVc},
};
use turbopack_dev_server::source::{
    ContentSource, ContentSourceContent, ContentSourceData, ContentSourceDataVary,
    ContentSourceResultVc, ContentSourceVc, NeededData, ProxyResult, RewriteVc,
};
use turbopack_node::execution_context::ExecutionContextVc;

use crate::{
    next_config::NextConfigVc,
    router::{route, RouterRequest, RouterResult},
};

#[turbo_tasks::value(shared)]
pub struct NextRouterContentSource {
    /// A wrapped content source from which we will fetch assets.
    inner: ContentSourceVc,
    execution_context: ExecutionContextVc,
    next_config: NextConfigVc,
    server_addr: ServerAddrVc,
}

#[turbo_tasks::value_impl]
impl NextRouterContentSourceVc {
    #[turbo_tasks::function]
    pub fn new(
        inner: ContentSourceVc,
        execution_context: ExecutionContextVc,
        next_config: NextConfigVc,
        server_addr: ServerAddrVc,
    ) -> NextRouterContentSourceVc {
        NextRouterContentSource {
            inner,
            execution_context,
            next_config,
            server_addr,
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
                method: true,
                raw_headers: true,
                raw_query: true,
                ..Default::default()
            },
        }
        .into(),
    )
}

/// If the route was resolved correctly by the Next.js router, this header will
/// be set to "1". Otherwise, it will be set to "0".
///
/// We need to differentiate between the two cases because the Next.js router
/// will apply rewrites and other URL modifications such as stripping the
/// base path.
///
/// For instance, if the base path is configured to be "/base", then we need to
/// handle "/base/page" and "/page" differently. However, once we go through the
/// Next.js router, both paths will be rewritten to "/page" and we won't be able
/// to tell which one was valid according to the router.
pub const TURBOPACK_NEXT_VALID_ROUTE: &str = "x-turbopack-valid-route";
pub const TURBOPACK_NEXT_VALID_ROUTE_TRUE: &str = "1";
pub const TURBOPACK_NEXT_VALID_ROUTE_FALSE: &str = "0";

fn invalid(path: &str, query: &str, next: ContentSourceVc) -> ContentSourceResultVc {
    ContentSourceResultVc::exact(
        ContentSourceContent::Rewrite(RewriteVc::new(
            format!("/{}?{}", path, query),
            vec![(
                TURBOPACK_NEXT_VALID_ROUTE.to_string(),
                TURBOPACK_NEXT_VALID_ROUTE_FALSE.to_string(),
            )],
            next,
        ))
        .cell()
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

        let ContentSourceData {
            method: Some(method),
            raw_headers: Some(raw_headers),
            raw_query: Some(raw_query),
            ..
        } = &*data else {
            return Ok(need_data(self_vc.into(), path))
        };

        let request = RouterRequest {
            pathname: format!("/{path}"),
            method: method.clone(),
            raw_headers: raw_headers.clone(),
            raw_query: raw_query.clone(),
        }
        .cell();

        let res = route(
            this.execution_context,
            request,
            this.next_config,
            this.server_addr,
        );
        dbg!(&(request.dbg().await?, res.await));

        let Ok(res) = res.await else {
            return Ok(invalid(path, raw_query, this.inner));
        };

        Ok(match &*res {
            // TODO: emit error
            RouterResult::Error => invalid(path, raw_query, this.inner),
            RouterResult::None => invalid(path, raw_query, this.inner),
            RouterResult::Rewrite(data) => {
                let mut headers = data.headers.clone();
                headers.push((
                    TURBOPACK_NEXT_VALID_ROUTE.to_string(),
                    TURBOPACK_NEXT_VALID_ROUTE_TRUE.to_string(),
                ));
                // TODO: We can't set response headers on the returned content.
                ContentSourceResultVc::exact(
                    ContentSourceContent::Rewrite(RewriteVc::new(
                        data.url.clone(),
                        headers,
                        this.inner,
                    ))
                    .cell()
                    .into(),
                )
            }
            RouterResult::FullMiddleware(data) => ContentSourceResultVc::exact(
                ContentSourceContent::HttpProxy(
                    ProxyResult {
                        status: data.headers.status_code,
                        headers: data.headers.headers.clone(),
                        body: data.body.clone().into(),
                    }
                    .cell(),
                )
                .cell()
                .into(),
            ),
        })
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
        let mut children = IndexSet::new();
        if let Some(inner) = IntrospectableVc::resolve_from(self.inner).await? {
            children.insert((StringVc::cell("inner".to_string()), inner));
        }
        Ok(IntrospectableChildrenVc::cell(children))
    }
}
