use anyhow::{anyhow, Result};
use turbo_tasks::{
    primitives::{OptionStringVc, StringVc},
    TryJoinIterExt, Value,
};
use turbopack_core::introspect::{Introspectable, IntrospectableChildrenVc, IntrospectableVc};

use super::{ContentSource, ContentSourceData, ContentSourceResultVc, ContentSourceVc};
use crate::source::ContentSourcesVc;

/// Binds different ContentSources to different subpaths. A fallback
/// ContentSource will serve all other subpaths.
#[turbo_tasks::value(shared)]
pub struct RouterContentSource {
    pub base_path: OptionStringVc,
    pub routes: Vec<(String, ContentSourceVc)>,
    pub fallback: ContentSourceVc,
}

impl RouterContentSource {
    async fn get_source<'s, 'a>(&'s self, path: &'a str) -> Result<(&'s ContentSourceVc, &'a str)> {
        let base_path = self.base_path.await?;
        let path = if let Some(base_path) = base_path.as_deref() {
            strip_base_path(path, base_path)?
        } else {
            path
        };
        for (route, source) in self.routes.iter() {
            if path.starts_with(route) {
                let path = &path[route.len()..];
                return Ok((source, path));
            }
        }
        Ok((&self.fallback, path))
    }
}

/// Strips a base path from a given path. The path must not start with a slash.
/// The base path must start with a slash and must not end with a slash, or be
/// the empty string.
fn strip_base_path<'a, 'b>(path: &'a str, base_path: &'b str) -> Result<&'a str> {
    if base_path.is_empty() {
        return Ok(path);
    }

    let base_path = base_path.strip_prefix('/').ok_or_else(|| {
        anyhow!(
            "invalid base path: base path must start with a slash, got {:?}",
            base_path
        )
    })?;
    Ok(path
        .strip_prefix(base_path)
        .map(|path| path.strip_prefix('/').unwrap_or(path))
        .unwrap_or(path))
}

#[turbo_tasks::value_impl]
impl ContentSource for RouterContentSource {
    #[turbo_tasks::function]
    async fn get(
        &self,
        path: &str,
        data: Value<ContentSourceData>,
    ) -> Result<ContentSourceResultVc> {
        let (source, path) = self.get_source(path).await?;
        Ok(source.get(path, data))
    }

    #[turbo_tasks::function]
    fn get_children(&self) -> ContentSourcesVc {
        let mut sources = Vec::with_capacity(self.routes.len() + 1);

        sources.extend(self.routes.iter().map(|r| r.1));
        sources.push(self.fallback);

        ContentSourcesVc::cell(sources)
    }
}

#[turbo_tasks::function]
fn introspectable_type() -> StringVc {
    StringVc::cell("router content source".to_string())
}

#[turbo_tasks::value_impl]
impl Introspectable for RouterContentSource {
    #[turbo_tasks::function]
    fn ty(&self) -> StringVc {
        introspectable_type()
    }

    #[turbo_tasks::function]
    async fn children(&self) -> Result<IntrospectableChildrenVc> {
        Ok(IntrospectableChildrenVc::cell(
            self.routes
                .iter()
                .cloned()
                .chain(std::iter::once((String::new(), self.fallback)))
                .map(|(path, source)| (StringVc::cell(path), source))
                .map(|(path, source)| async move {
                    Ok(IntrospectableVc::resolve_from(source)
                        .await?
                        .map(|i| (path, i)))
                })
                .try_join()
                .await?
                .into_iter()
                .flatten()
                .collect(),
        ))
    }
}
