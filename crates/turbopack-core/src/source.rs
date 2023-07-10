use crate::asset::{Asset, AssetVc};

/// (Unparsed) Source Code. Source Code is processed into [Module]s by the
/// [AssetContext]. All [Source]s have content and an identifier.
#[turbo_tasks::value_trait]
pub trait Source: Asset {}

#[turbo_tasks::value(transparent)]
pub struct OptionSource(Option<SourceVc>);
