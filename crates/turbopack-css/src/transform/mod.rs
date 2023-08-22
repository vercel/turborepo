use std::sync::Arc;

use anyhow::Result;
use lightningcss::stylesheet::StyleSheet;
use swc_core::common::SourceMap;

#[turbo_tasks::value(serialization = "auto_for_input")]
#[derive(PartialOrd, Ord, Hash, Debug, Copy, Clone)]
pub enum CssInputTransform {
    Nested,
    Custom,
}

#[turbo_tasks::value(transparent, serialization = "auto_for_input")]
#[derive(Debug, PartialOrd, Ord, Hash, Clone)]
pub struct CssInputTransforms(Vec<CssInputTransform>);

pub struct TransformContext<'a> {
    pub source_map: &'a Arc<SourceMap>,
}

impl CssInputTransform {
    pub async fn apply(
        &self,
        stylesheet: &mut StyleSheet<'static, 'static>,
        &TransformContext { source_map: _ }: &TransformContext<'_>,
    ) -> Result<()> {
        match *self {
            CssInputTransform::Nested => {
                // TODO(kdy1): Apply nested transform using lightningcss
                // stylesheet.visit_mut_with(&mut
                // swc_core::css::compat::compiler::Compiler::new(
                //     swc_core::css::compat::compiler::Config {
                //         process:
                // swc_core::css::compat::feature::Features::NESTING,
                //     },
                // ));
            }
            CssInputTransform::Custom => todo!(),
        }
        Ok(())
    }
}
