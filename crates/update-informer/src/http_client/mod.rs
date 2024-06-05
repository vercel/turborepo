use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::Result;

#[cfg(feature = "ureq")]
mod ureq;
#[cfg(all(feature = "ureq", not(feature = "reqwest")))]
pub use crate::http_client::ureq::UreqHttpClient;
#[cfg(all(feature = "ureq", not(feature = "reqwest")))]
pub type DefaultHttpClient = UreqHttpClient;

#[cfg(feature = "reqwest")]
mod reqwest;
#[cfg(all(feature = "reqwest", not(feature = "ureq")))]
pub use crate::http_client::reqwest::ReqwestHttpClient;
#[cfg(all(feature = "reqwest", not(feature = "ureq")))]
pub type DefaultHttpClient = ReqwestHttpClient;

#[cfg(all(not(feature = "ureq"), not(feature = "reqwest")))]
mod undefined;
#[cfg(all(not(feature = "ureq"), not(feature = "reqwest")))]
pub use crate::http_client::undefined::UndefinedHttpClient;
#[cfg(all(not(feature = "ureq"), not(feature = "reqwest")))]
pub type DefaultHttpClient = UndefinedHttpClient;

mod headers;
pub use headers::HeaderMap;

/// An HTTP client to send requests to the registry.
pub struct GenericHttpClient<'a, T: HttpClient> {
    _inner: T,
    timeout: Duration,
    headers: HeaderMap<'a>,
}

pub(crate) fn new<'a, T: HttpClient>(client: T, timeout: Duration) -> GenericHttpClient<'a, T> {
    GenericHttpClient {
        _inner: client,
        timeout,
        headers: HeaderMap::new(),
    }
}

impl<'a, T: HttpClient> GenericHttpClient<'a, T> {
    pub fn add_header(mut self, key: &'a str, value: &'a str) -> Self {
        self.headers.add(key, value);
        self
    }

    pub fn get<D: DeserializeOwned>(self, url: &str) -> Result<D> {
        T::get(url, self.timeout, self.headers)
    }
}

pub trait HttpClient {
    fn get<T: DeserializeOwned>(url: &str, timeout: Duration, headers: HeaderMap) -> Result<T>;
}
