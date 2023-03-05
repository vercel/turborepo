use anyhow::Result;

use crate::{asset::AssetVc, source_map::SourceMapVc};

#[turbo_tasks::value_trait]
pub trait SourceTransform {
    fn transform(&self, source: SourceTransformedVc) -> SourceTransformedVc;
}

#[turbo_tasks::value]
pub struct SourceTransformed {
    pub source: AssetVc,
    pub source_map: Option<SourceMapVc>,
}

#[turbo_tasks::value_impl]
impl SourceTransformedVc {
    #[turbo_tasks::function]
    pub fn new(source: AssetVc, source_map: Option<SourceMapVc>) -> Self {
        SourceTransformed { source, source_map }.cell()
    }
}

#[turbo_tasks::value(transparent)]
pub struct SourceTransforms(Vec<SourceTransformVc>);

#[turbo_tasks::value_impl]
impl SourceTransformsVc {
    #[turbo_tasks::function]
    pub async fn transform(self, original: SourceTransformedVc) -> Result<SourceTransformedVc> {
        Ok(self.await?.iter().fold(original, |original, transform| {
            transform.transform(original)
        }))
    }
}
