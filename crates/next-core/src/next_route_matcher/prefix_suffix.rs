use serde::{Deserialize, Serialize};
use turbo_tasks::Value;
use turbopack_dev_server::source::ContentSourceData;
use turbopack_node::route_matcher::{MatchResultVc, RouteMatcher};

/// A composite route matcher that matches a path if it has a given prefix and
/// suffix.
#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct PrefixSuffixMatcher<T>
where
    T: RouteMatcher,
{
    prefix: String,
    suffix: String,
    inner: T,
}

impl<T> PrefixSuffixMatcher<T>
where
    T: RouteMatcher,
{
    /// Creates a new [PrefixSuffixMatcher].
    pub fn new(prefix: String, suffix: String, inner: T) -> Self {
        Self {
            prefix,
            suffix,
            inner,
        }
    }

    fn strip_prefix_and_suffix<'a, 'b>(&'a self, path: &'b str) -> Option<&'b str> {
        path.strip_prefix(self.prefix.as_str())?
            .strip_suffix(self.suffix.as_str())
    }
}

impl<T> RouteMatcher for PrefixSuffixMatcher<T>
where
    T: RouteMatcher,
{
    fn match_params(&self, path: &str, data: Value<ContentSourceData>) -> MatchResultVc {
        if let Some(path) = self.strip_prefix_and_suffix(path) {
            self.inner.match_params(path, data)
        } else {
            MatchResultVc::not_found()
        }
    }
}
