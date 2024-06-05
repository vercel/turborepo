use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::{
    http_client::{HeaderMap, HttpClient},
    Result,
};

pub struct UreqHttpClient;

impl HttpClient for UreqHttpClient {
    fn get<T: DeserializeOwned>(url: &str, timeout: Duration, headers: HeaderMap) -> Result<T> {
        let mut req = ureq::agent().get(url).timeout(timeout);

        for (header, value) in headers {
            req = req.set(header, value);
        }

        let json = req.call()?.into_json()?;

        Ok(json)
    }
}
