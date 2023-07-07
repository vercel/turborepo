use crate::asset::{Asset, AssetVc};

#[turbo_tasks::value_trait]
pub trait Module: Asset {}
