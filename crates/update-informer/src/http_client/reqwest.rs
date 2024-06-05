use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::{
    http_client::{HeaderMap, HttpClient},
    Result,
};

pub struct ReqwestHttpClient;

impl HttpClient for ReqwestHttpClient {
    fn get<T: DeserializeOwned>(url: &str, timeout: Duration, headers: HeaderMap) -> Result<T> {
        let mut req = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()?
            .get(url);

        for (key, value) in headers {
            req = req.header(key, value);
        }

        let json = req.send()?.json()?;

        Ok(json)
    }
}
