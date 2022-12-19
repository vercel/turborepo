use turbo_tasks::primitives::StringVc;
use turbopack_core::asset::{Asset, AssetVc};

#[turbo_tasks::value_trait]
pub trait DevErrorHtmlAsset: Asset {
    fn error(&self, exit_code: Option<i32>, error: StringVc) -> AssetVc;
}
