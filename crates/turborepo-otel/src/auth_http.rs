use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use turborepo_types::Credential;

#[derive(Debug)]
pub(crate) struct CredentialHttpClient {
    client: turborepo_api_client::HttpClient,
    credential: Option<Credential>,
    timeout: std::time::Duration,
}

impl CredentialHttpClient {
    pub(crate) fn new(
        credential: Option<Credential>,
        timeout: std::time::Duration,
    ) -> Result<Self, turborepo_api_client::Error> {
        Ok(Self {
            client: turborepo_api_client::APIClient::build_http_client(Some(timeout))?,
            credential,
            timeout,
        })
    }
}

#[async_trait::async_trait]
impl HttpClient for CredentialHttpClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let mut request: reqwest::Request = request.try_into()?;
        *request.timeout_mut() = Some(self.timeout);
        if let Some(credential) = &self.credential {
            turborepo_api_client::validate_credential_destination(credential, request.url())?;
            let mut authorization =
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", credential.expose()))?;
            authorization.set_sensitive(true);
            request
                .headers_mut()
                .insert(reqwest::header::AUTHORIZATION, authorization);
        }
        let mut response = self
            .client
            .execute_with_credential(request, self.credential.as_ref())
            .await?
            .error_for_status()?;
        let headers = std::mem::take(response.headers_mut());
        let mut result = Response::builder()
            .status(response.status())
            .body(response.bytes().await?)?;
        *result.headers_mut() = headers;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_protected_injection_fails_closed_before_exporter_initialization() {
        let config = crate::Config {
            endpoint: "https://api.vercel.com/v1/metrics".into(),
            protocol: crate::Protocol::Grpc,
            headers: Default::default(),
            timeout: std::time::Duration::from_secs(10),
            interval: std::time::Duration::from_secs(15),
            resource_attributes: Default::default(),
            metrics: Default::default(),
        };
        let error = crate::Handle::try_new_with_credential(
            config,
            Some(Credential::saved_vercel("synthetic-otel-sentinel".into())),
        )
        .unwrap_err();
        assert!(matches!(error, crate::Error::ProtectedGrpcCredential));
    }

    #[tokio::test]
    async fn exporter_checks_actual_destination_and_preserves_generic_credentials() {
        let server = httpmock::MockServer::start_async().await;
        let received = server
            .mock_async(|when, then| {
                when.header("authorization", "Bearer synthetic-otel-sentinel");
                then.status(200);
            })
            .await;
        let request = || {
            Request::builder()
                .method("POST")
                .uri(server.url("/v1/metrics"))
                .body(Bytes::new())
                .unwrap()
        };
        let protected = CredentialHttpClient::new(
            Some(Credential::saved_vercel("synthetic-otel-sentinel".into())),
            std::time::Duration::from_secs(10),
        )
        .unwrap();
        assert!(protected.send_bytes(request()).await.is_err());
        received.assert_calls_async(0).await;
        let generic = CredentialHttpClient::new(
            Some(Credential::generic("synthetic-otel-sentinel".into())),
            std::time::Duration::from_secs(10),
        )
        .unwrap();
        assert!(generic.send_bytes(request()).await.is_ok());
        received.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn exporter_follows_generic_credential_redirects() {
        let server = httpmock::MockServer::start_async().await;
        let target = server
            .mock_async(|when, then| {
                when.path("/redirected")
                    .header("authorization", "Bearer synthetic-otel-sentinel");
                then.status(200);
            })
            .await;
        server
            .mock_async(|when, then| {
                when.path("/v1/metrics");
                then.status(307)
                    .header("location", server.url("/redirected"));
            })
            .await;
        let client = CredentialHttpClient::new(
            Some(Credential::generic("synthetic-otel-sentinel".into())),
            std::time::Duration::from_secs(10),
        )
        .unwrap();
        let request = Request::builder()
            .method("POST")
            .uri(server.url("/v1/metrics"))
            .body(Bytes::new())
            .unwrap();
        assert_eq!(client.send_bytes(request).await.unwrap().status(), 200);
        target.assert_calls_async(1).await;
    }
}
