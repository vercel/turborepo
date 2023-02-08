use serde::{Deserialize, Serialize};
use turbo_tasks::Value;
use turbopack_dev_server::source::{ContentSourceData, ParamsVc};
use turbopack_node::route_matcher::{MatchResultVc, RouteMatcher};

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct AllMatch;

impl RouteMatcher for AllMatch {
    fn match_params(&self, _path: &str, _data: Value<ContentSourceData>) -> MatchResultVc {
        MatchResultVc::match_params(ParamsVc::empty())
    }
}
