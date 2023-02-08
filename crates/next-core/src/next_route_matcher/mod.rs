use anyhow::{bail, Result};
use turbo_tasks::{primitives::StringVc, Value};
use turbopack_dev_server::source::{
    ContentSourceData, ContentSourceDataFilter, ContentSourceDataVary, ParamsVc,
};
use turbopack_node::route_matcher::{MatchResultVc, RouteMatcher, RouteMatcherVc};

use self::{
    all::AllMatch,
    path_regex::{PathRegex, PathRegexBuilder},
    prefix_suffix::PrefixSuffixMatcher,
};
use crate::router_source::{
    TURBOPACK_NEXT_VALID_ROUTE, TURBOPACK_NEXT_VALID_ROUTE_FALSE, TURBOPACK_NEXT_VALID_ROUTE_TRUE,
};

mod all;
mod path_regex;
mod prefix_suffix;

/// A route matcher that matches a path against an exact route.
#[turbo_tasks::value]
pub(crate) struct NextExactMatcher {
    path: StringVc,
}

#[turbo_tasks::value_impl]
impl NextExactMatcherVc {
    #[turbo_tasks::function]
    pub async fn new(path: StringVc) -> Result<Self> {
        Ok(Self::cell(NextExactMatcher { path }))
    }
}

#[turbo_tasks::value_impl]
impl RouteMatcher for NextExactMatcher {
    #[turbo_tasks::function]
    async fn match_params(
        &self,
        path: &str,
        _data: Value<ContentSourceData>,
    ) -> Result<MatchResultVc> {
        Ok(if path == *self.path.await? {
            MatchResultVc::match_params(ParamsVc::empty())
        } else {
            MatchResultVc::not_found()
        })
    }
}

/// A route matcher that matches a path against a route regex.
#[turbo_tasks::value]
pub(crate) struct NextParamsMatcher {
    #[turbo_tasks(trace_ignore)]
    matcher: PathRegex,
}

#[turbo_tasks::value_impl]
impl NextParamsMatcherVc {
    #[turbo_tasks::function]
    pub async fn new(path: StringVc) -> Result<Self> {
        Ok(Self::cell(NextParamsMatcher {
            matcher: build_path_regex(path.await?.as_str())?,
        }))
    }
}

/// Checks whether the given route is a valid Next.js route according to the
/// Next.js router.
///
/// Only valid routes will be served by the Next.js page and app content
/// sources.
fn lookup_turbopack_header<T>(
    path: &str,
    data: Value<ContentSourceData>,
    matcher: &T,
) -> Result<MatchResultVc>
where
    T: RouteMatcher,
{
    Ok(if let Some(headers) = &data.headers {
        if let Some(found) = headers.get(TURBOPACK_NEXT_VALID_ROUTE) {
            match found.as_str() {
                Some(TURBOPACK_NEXT_VALID_ROUTE_TRUE) => matcher.match_params(path, data),
                Some(TURBOPACK_NEXT_VALID_ROUTE_FALSE) => MatchResultVc::not_found(),
                Some(value) => {
                    bail!(
                        "expected header {} to be set to {} or {}, but found {}",
                        TURBOPACK_NEXT_VALID_ROUTE,
                        TURBOPACK_NEXT_VALID_ROUTE_TRUE,
                        TURBOPACK_NEXT_VALID_ROUTE_FALSE,
                        value
                    );
                }
                None => {
                    bail!(
                        "expected header {} to be set to {} or {}, but found an invalid value",
                        TURBOPACK_NEXT_VALID_ROUTE,
                        TURBOPACK_NEXT_VALID_ROUTE_TRUE,
                        TURBOPACK_NEXT_VALID_ROUTE_FALSE,
                    );
                }
            }
        } else {
            bail!("expected header {} to be set", TURBOPACK_NEXT_VALID_ROUTE);
        }
    } else {
        MatchResultVc::need_data(Value::new(ContentSourceDataVary {
            headers: Some(ContentSourceDataFilter::Subset(
                [TURBOPACK_NEXT_VALID_ROUTE.to_string()].into(),
            )),
            ..Default::default()
        }))
    })
}

#[turbo_tasks::value_impl]
impl RouteMatcher for NextParamsMatcher {
    #[turbo_tasks::function]
    fn match_params(&self, path: &str, data: Value<ContentSourceData>) -> Result<MatchResultVc> {
        lookup_turbopack_header(path, data, &self.matcher)
    }
}

/// A route matcher that strips a prefix and a suffix from a path before
/// matching it against a route regex.
#[turbo_tasks::value]
pub(crate) struct NextPrefixSuffixParamsMatcher {
    #[turbo_tasks(trace_ignore)]
    matcher: PrefixSuffixMatcher<PathRegex>,
}

#[turbo_tasks::value_impl]
impl NextPrefixSuffixParamsMatcherVc {
    /// Converts a filename within the server root into a regular expression
    /// with named capture groups for every dynamic segment.
    #[turbo_tasks::function]
    pub async fn new(path: StringVc, prefix: &str, suffix: &str) -> Result<Self> {
        Ok(Self::cell(NextPrefixSuffixParamsMatcher {
            matcher: PrefixSuffixMatcher::new(
                prefix.to_string(),
                suffix.to_string(),
                build_path_regex(path.await?.as_str())?,
            ),
        }))
    }
}

#[turbo_tasks::value_impl]
impl RouteMatcher for NextPrefixSuffixParamsMatcher {
    #[turbo_tasks::function]
    fn match_params(&self, path: &str, data: Value<ContentSourceData>) -> Result<MatchResultVc> {
        lookup_turbopack_header(path, data, &self.matcher)
    }
}

/// A route matcher that matches against all paths.
#[turbo_tasks::value]
pub(crate) struct NextFallbackMatcher {
    #[turbo_tasks(trace_ignore)]
    matcher: AllMatch,
}

#[turbo_tasks::value_impl]
impl NextFallbackMatcherVc {
    #[turbo_tasks::function]
    pub fn new() -> Self {
        Self::cell(NextFallbackMatcher { matcher: AllMatch })
    }
}

#[turbo_tasks::value_impl]
impl RouteMatcher for NextFallbackMatcher {
    #[turbo_tasks::function]
    fn match_params(&self, path: &str, data: Value<ContentSourceData>) -> MatchResultVc {
        self.matcher.match_params(path, data)
    }
}

/// Converts a filename within the server root into a regular expression
/// with named capture groups for every dynamic segment.
fn build_path_regex(path: &str) -> Result<PathRegex> {
    let mut path_regex = PathRegexBuilder::new();
    for segment in path.split('/') {
        if let Some(segment) = segment.strip_prefix('[') {
            if let Some(segment) = segment.strip_prefix("[...") {
                if let Some((placeholder, rem)) = segment.split_once("]]") {
                    path_regex.push_optional_catch_all(placeholder, rem);
                } else {
                    bail!(
                        "path ({}) contains '[[' without matching ']]' at '[[...{}'",
                        path,
                        segment
                    );
                }
            } else if let Some(segment) = segment.strip_prefix("...") {
                if let Some((placeholder, rem)) = segment.split_once(']') {
                    path_regex.push_catch_all(placeholder, rem);
                } else {
                    bail!(
                        "path ({}) contains '[' without matching ']' at '[...{}'",
                        path,
                        segment
                    );
                }
            } else if let Some((placeholder, rem)) = segment.split_once(']') {
                path_regex.push_dynamic_segment(placeholder, rem);
            } else {
                bail!(
                    "path ({}) contains '[' without matching ']' at '[{}'",
                    path,
                    segment
                );
            }
        } else {
            path_regex.push_static_segment(segment);
        }
    }
    path_regex.build()
}
