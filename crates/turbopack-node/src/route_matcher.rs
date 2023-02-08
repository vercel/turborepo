use turbo_tasks::Value;
use turbopack_dev_server::source::{ContentSourceData, ContentSourceDataVary, ParamsVc};

#[turbo_tasks::value]
pub enum MatchResult {
    NotFound,
    NeedData(ContentSourceDataVary),
    MatchParams(ParamsVc),
}

#[turbo_tasks::value_impl]
impl MatchResultVc {
    #[turbo_tasks::function]
    pub fn not_found() -> MatchResultVc {
        MatchResult::NotFound.cell()
    }

    #[turbo_tasks::function]
    pub fn match_params(params: ParamsVc) -> MatchResultVc {
        MatchResult::MatchParams(params).cell()
    }

    #[turbo_tasks::function]
    pub fn need_data(vary: Value<ContentSourceDataVary>) -> MatchResultVc {
        MatchResult::NeedData(vary.into_value()).cell()
    }
}

/// Extracts parameters from a URL path.
#[turbo_tasks::value_trait]
pub trait RouteMatcher {
    /// Returns whether the given path is a match for the route.
    fn match_params(&self, path: &str, data: Value<ContentSourceData>) -> MatchResultVc;
}
