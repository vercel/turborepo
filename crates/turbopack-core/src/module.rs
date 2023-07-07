use crate::asset::{Asset, AssetVc};

#[turbo_tasks::value_trait]
pub trait Module: Asset {}

#[turbo_tasks::value(transparent)]
pub struct OptionModule(Option<ModuleVc>);
