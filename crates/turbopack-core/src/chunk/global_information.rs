use std::collections::HashMap;

use turbo_tasks::{RcStr, Vc};

use super::ModuleId;
use crate::ident::AssetIdent;

#[turbo_tasks::value]
#[derive(Clone, Debug)]
pub struct GlobalInformation {
    pub test_str: Vc<RcStr>,
    pub module_id_map: HashMap<AssetIdent, ModuleId>,
}

impl GlobalInformation {
    pub fn get_module_id(&self, asset_ident: &AssetIdent) -> ModuleId {
        self.module_id_map.get(asset_ident).cloned().expect(
            "No module ID found for the given asset identifier. This is an internal Turbopack \
             error. Please report it.",
        )
    }
}

#[turbo_tasks::value(transparent)]
pub struct OptionGlobalInformation(Option<GlobalInformation>);
