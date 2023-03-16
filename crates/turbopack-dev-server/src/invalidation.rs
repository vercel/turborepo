use std::borrow::Cow;

use hyper::{Method, Uri};
use indexmap::IndexSet;
use turbo_tasks::{InvalidationReason, InvalidationReasonType};

pub struct ServerRequest {
    pub method: Method,
    pub uri: Uri,
}

impl InvalidationReason for ServerRequest {
    fn description(&self) -> Cow<'static, str> {
        format!("{} {}", self.method, self.uri.path()).into()
    }
    fn merge_info(&self) -> Option<(&'static dyn InvalidationReasonType, Cow<'static, str>)> {
        Some((&SERVER_REQUEST_TYPE, self.description()))
    }
}

struct ServerRequestType {
    _non_zero_sized: u8,
}

static SERVER_REQUEST_TYPE: ServerRequestType = ServerRequestType { _non_zero_sized: 0 };

impl InvalidationReasonType for ServerRequestType {
    fn description(&self, merge_data: &IndexSet<Cow<'static, str>>) -> Cow<'static, str> {
        let example = merge_data
            .into_iter()
            .reduce(|a, b| if b.len() < a.len() { b } else { a })
            .unwrap();
        format!("{} requests (e. g. {})", merge_data.len(), example).into()
    }
}
