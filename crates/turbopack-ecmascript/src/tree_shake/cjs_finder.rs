use swc_core::ecma::ast::*;

use crate::TURBOPACK_HELPER;

pub fn should_skip_tree_shaking(m: &Program) -> bool {
    if let Program::Module(m) = m {
        if m.body.iter().any(|item| {
            if let ModuleItem::ModuleDecl(ModuleDecl::Import(ImportDecl {
                with: Some(with), ..
            })) = item
            {
                let with = with.as_import_with();
                if let Some(with) = with {
                    for item in with.values.iter() {
                        if item.key.sym == *TURBOPACK_HELPER {
                            return false;
                        }
                    }
                }
            }

            return true;
        }) {
            return false;
        }
    }

    true
}
