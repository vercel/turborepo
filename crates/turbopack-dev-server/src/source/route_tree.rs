use std::{collections::HashMap, mem::replace};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use turbo_tasks::{trace::TraceRawVcs, TaskInput, TryJoinIterExt};

use super::{GetContentSourceContentVc, GetContentSourceContentsVc};

#[derive(TaskInput, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TraceRawVcs)]
pub enum FinalSegment {
    CatchAll,
    Fallback,
    NotFound,
}

#[derive(TaskInput, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TraceRawVcs)]
pub enum BaseSegment {
    Static(String),
    Dynamic,
}

impl BaseSegment {
    pub fn from_static_pathname(str: &str) -> impl Iterator<Item = BaseSegment> + '_ {
        str.split('/')
            .filter(|s| !s.is_empty())
            .map(|s| BaseSegment::Static(s.to_string()))
    }
}

#[turbo_tasks::value]
#[derive(Default, Clone)]
pub struct RouteTree {
    pub base: Vec<BaseSegment>,
    pub sources: Vec<GetContentSourceContentVc>,
    pub static_segments: HashMap<String, RouteTreeVc>,
    pub dynamic_segments: Vec<RouteTreeVc>,
    pub catch_all_sources: Vec<GetContentSourceContentVc>,
    pub fallback_sources: Vec<GetContentSourceContentVc>,
    pub not_found_sources: Vec<GetContentSourceContentVc>,
}

impl RouteTree {
    pub fn new_route(
        base_segments: Vec<BaseSegment>,
        final_segment: Option<FinalSegment>,
        source: GetContentSourceContentVc,
    ) -> Self {
        match final_segment {
            None => Self {
                base: base_segments,
                sources: vec![source],
                ..Default::default()
            },
            Some(FinalSegment::CatchAll) => Self {
                base: base_segments,
                catch_all_sources: vec![source],
                ..Default::default()
            },
            Some(FinalSegment::Fallback) => Self {
                base: base_segments,
                fallback_sources: vec![source],
                ..Default::default()
            },
            Some(FinalSegment::NotFound) => Self {
                base: base_segments,
                not_found_sources: vec![source],
                ..Default::default()
            },
        }
    }

    pub async fn flat_merge(&mut self, others: impl IntoIterator<Item = &Self> + '_) -> Result<()> {
        let mut static_segments = HashMap::new();
        for other in others {
            debug_assert_eq!(self.base, other.base);
            self.sources.extend(other.sources.iter().copied());
            self.catch_all_sources
                .extend(other.catch_all_sources.iter().copied());
            self.fallback_sources
                .extend(other.fallback_sources.iter().copied());
            self.not_found_sources
                .extend(other.not_found_sources.iter().copied());
            for (key, value) in other.static_segments.iter() {
                if let Some((key, self_value)) = self.static_segments.remove_entry(key) {
                    static_segments.insert(key, vec![self_value, *value]);
                } else if let Some(list) = static_segments.get_mut(key) {
                    list.push(*value);
                } else {
                    static_segments.insert(key.clone(), vec![*value]);
                }
            }
            self.dynamic_segments
                .extend(other.dynamic_segments.iter().copied());
        }
        self.static_segments.extend(
            static_segments
                .into_iter()
                .map(|(key, value)| async {
                    Ok((
                        key,
                        if value.len() == 1 {
                            value.into_iter().next().unwrap()
                        } else {
                            RouteTreeVc::merge(value).resolve().await?
                        },
                    ))
                })
                .try_join()
                .await?,
        );
        Ok(())
    }

    fn prepend_base(&mut self, segments: Vec<BaseSegment>) {
        self.base.splice(..0, segments);
    }
}

#[turbo_tasks::value_impl]
impl RouteTreeVc {
    #[turbo_tasks::function]
    pub fn empty() -> RouteTreeVc {
        RouteTree::default().cell()
    }

    #[turbo_tasks::function]
    pub fn new_route(
        base_segments: Vec<BaseSegment>,
        final_segment: Option<FinalSegment>,
        source: GetContentSourceContentVc,
    ) -> Self {
        RouteTree::new_route(base_segments, final_segment, source).cell()
    }

    #[turbo_tasks::function]
    pub async fn get(self, path: &str) -> Result<GetContentSourceContentsVc> {
        let RouteTree {
            base,
            sources,
            static_segments,
            dynamic_segments,
            catch_all_sources,
            fallback_sources,
            not_found_sources,
        } = &*self.await?;
        let mut segments = path.split('/');
        for base in base.iter() {
            let Some(segment) = segments.next() else {
                return Ok(GetContentSourceContentsVc::cell(vec![]));
            };
            match base {
                BaseSegment::Static(str) => {
                    if str != segment {
                        return Ok(GetContentSourceContentsVc::cell(vec![]));
                    }
                }
                BaseSegment::Dynamic => {
                    // always matching
                }
            }
        }

        let Some(segment) = segments.next() else {
            return Ok(GetContentSourceContentsVc::cell(sources.clone()))
        };
        let remainder = segments.remainder().unwrap_or("");
        if let Some(tree) = static_segments.get(segment) {
            return Ok(tree.get(remainder));
        }
        let mut results = Vec::new();
        for tree in dynamic_segments.iter() {
            results.extend(tree.get(remainder).await?.iter().copied());
        }
        results.extend(catch_all_sources.iter().copied());
        results.extend(fallback_sources.iter().copied());
        results.extend(not_found_sources.iter().copied());
        Ok(GetContentSourceContentsVc::cell(results))
    }

    #[turbo_tasks::function]
    pub async fn merge(trees: Vec<RouteTreeVc>) -> Result<RouteTreeVc> {
        if trees.is_empty() {
            return Ok(RouteTree::default().cell());
        }
        if trees.len() == 1 {
            return Ok(trees.into_iter().next().unwrap());
        }

        // Find common base
        let mut tree_values = trees.iter().try_join().await?;
        let mut common_base = 0;
        let last_tree = tree_values.pop().unwrap();
        while common_base < last_tree.base.len() {
            for tree in tree_values.iter() {
                if tree.base.len() <= common_base {
                    break;
                }
                if tree.base[common_base] != last_tree.base[common_base] {
                    break;
                }
                common_base += 1;
            }
        }
        tree_values.push(last_tree);

        // Normalize bases to common base
        let mut trees = trees;
        for (i, tree) in trees.iter_mut().enumerate() {
            if tree_values[i].base.len() > common_base {
                *tree = tree.with_base_len(common_base);
            }
        }

        // Flat merge trees
        let tree_values = trees.into_iter().try_join().await?;
        let mut iter = tree_values.iter().map(|rr| &**rr);
        let mut merged = iter.next().unwrap().clone();
        merged.flat_merge(iter).await?;

        Ok(merged.cell())
    }

    #[turbo_tasks::function]
    pub async fn with_prepended_base(
        self_vc: RouteTreeVc,
        segments: Vec<BaseSegment>,
    ) -> Result<RouteTreeVc> {
        let mut this = self_vc.await?.clone_value();
        this.prepend_base(segments);
        Ok(this.cell())
    }

    #[turbo_tasks::function]
    async fn with_base_len(self_vc: RouteTreeVc, base_len: usize) -> Result<RouteTreeVc> {
        let this = self_vc.await?;
        if this.base.len() > base_len {
            let mut inner = this.clone_value();
            let mut drain = inner.base.drain(base_len..);
            let selector_segment = drain.next().unwrap();
            let inner_base = drain.collect();
            let base = replace(&mut inner.base, inner_base);
            match selector_segment {
                BaseSegment::Static(value) => Ok(RouteTree {
                    base,
                    static_segments: HashMap::from([(value, inner.cell())]),
                    ..Default::default()
                }
                .cell()),
                BaseSegment::Dynamic => Ok(RouteTree {
                    base,
                    dynamic_segments: vec![inner.cell()],
                    ..Default::default()
                }
                .cell()),
            }
        } else {
            Ok(self_vc)
        }
    }

    #[turbo_tasks::function]
    pub async fn map_routes(self, mapper: MapGetContentSourceContentVc) -> Result<Self> {
        let mut this = self.await?.clone_value();
        let RouteTree {
            base: _,
            static_segments,
            dynamic_segments,
            sources,
            catch_all_sources,
            fallback_sources,
            not_found_sources,
        } = &mut this;
        sources
            .iter_mut()
            .for_each(|s| *s = mapper.map_get_content(*s));
        catch_all_sources
            .iter_mut()
            .for_each(|s| *s = mapper.map_get_content(*s));
        fallback_sources
            .iter_mut()
            .for_each(|s| *s = mapper.map_get_content(*s));
        not_found_sources
            .iter_mut()
            .for_each(|s| *s = mapper.map_get_content(*s));
        static_segments
            .values_mut()
            .for_each(|r| *r = r.map_routes(mapper));
        dynamic_segments
            .iter_mut()
            .for_each(|r| *r = r.map_routes(mapper));
        Ok(this.cell())
    }
}

#[turbo_tasks::value_trait]
pub trait MapGetContentSourceContent {
    fn map_get_content(&self, get_content: GetContentSourceContentVc) -> GetContentSourceContentVc;
}
