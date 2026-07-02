use std::sync::Arc;

use async_graphql::{EmptyMutation, EmptySubscription, Schema};
use async_graphql_axum::GraphQL;
use axum::{
    extract::Request,
    http::{header::HOST, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::get,
    Router,
};
use tokio::net::TcpListener;

use crate::{graphiql, QueryRun, RepositoryQuery};

fn is_allowed_host(host: &str) -> bool {
    let host = match host.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') && port.chars().all(|c| c.is_ascii_digit()) => {
            host
        }
        Some(_) => return false,
        None => host,
    };

    matches!(host, "127.0.0.1" | "localhost")
}

async fn require_localhost_host(request: Request, next: Next) -> Result<Response, StatusCode> {
    let allowed = request
        .headers()
        .get(HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(is_allowed_host);

    if !allowed {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(next.run(request).await)
}

pub async fn run_server(run: Arc<dyn QueryRun>) -> std::io::Result<()> {
    let turbo_query = RepositoryQuery::new(run);

    let schema = Schema::new(turbo_query, EmptyMutation, EmptySubscription);
    let app = Router::new()
        .route("/", get(graphiql).post_service(GraphQL::new(schema)))
        .layer(middleware::from_fn(require_localhost_host));

    axum::serve(TcpListener::bind("127.0.0.1:8000").await?, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_allowed_host;

    #[test]
    fn allows_localhost_hosts() {
        assert!(is_allowed_host("127.0.0.1:8000"));
        assert!(is_allowed_host("localhost:8000"));
    }

    #[test]
    fn rejects_non_localhost_hosts() {
        assert!(!is_allowed_host("example.com:8000"));
        assert!(!is_allowed_host("localhost.example.com:8000"));
        assert!(!is_allowed_host("127.0.0.1:8000:extra"));
        assert!(!is_allowed_host("127.0.0.1:not-a-port"));
    }
}
