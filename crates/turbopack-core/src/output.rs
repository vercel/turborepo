use crate::asset::{Asset, AssetVc};

/// An asset that should be outputted, e. g. written to disk or served from a
/// server.
#[turbo_tasks::value_trait]
pub trait OutputAsset: Asset {}
