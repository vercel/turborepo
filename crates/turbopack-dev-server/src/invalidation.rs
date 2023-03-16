use std::fmt::{Display, Formatter};

use hyper::{Method, Uri};
use indexmap::IndexSet;
use turbo_tasks::{util::StaticOrArc, InvalidationReason, InvalidationReasonKind};

#[derive(PartialEq, Eq, Hash)]
pub struct ServerRequest {
    pub method: Method,
    pub uri: Uri,
}

impl InvalidationReason for ServerRequest {
    fn kind(&self) -> Option<StaticOrArc<dyn InvalidationReasonKind>> {
        Some(StaticOrArc::Static(&SERVER_REQUEST_TYPE))
    }
}

impl Display for ServerRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.method, self.uri.path())
    }
}

#[derive(PartialEq, Eq, Hash)]
struct ServerRequestType;

static SERVER_REQUEST_TYPE: ServerRequestType = ServerRequestType;

impl InvalidationReasonKind for ServerRequestType {
    fn fmt(
        &self,
        reasons: &IndexSet<StaticOrArc<dyn InvalidationReason>>,
        f: &mut Formatter<'_>,
    ) -> std::fmt::Result {
        let example = reasons
            .into_iter()
            .map(|reason| reason.as_any().downcast_ref::<ServerRequest>().unwrap())
            .reduce(|a, b| {
                if b.uri.path().len() < a.uri.path().len() {
                    b
                } else {
                    a
                }
            })
            .unwrap();
        write!(
            f,
            "{} requests (e. g. {} {})",
            reasons.len(),
            example.method,
            example.uri.path()
        )
    }
}
