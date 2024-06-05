use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::{
    http_client::{HeaderMap, HttpClient},
    Result,
};

pub struct UndefinedHttpClient;

impl HttpClient for UndefinedHttpClient {
    fn get<T: DeserializeOwned>(_url: &str, _timeout: Duration, _headers: HeaderMap) -> Result<T> {
        panic!("choose HTTP client (ureq or reqwest) or implement your own");
    }
}
