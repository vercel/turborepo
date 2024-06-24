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
                            // Skip tree shaking if the import is from turbopack-helper
                            return false;
                        }
                    }
                }
            }

            // We don't have logic to tree shake export * from
            if let ModuleItem::ModuleDecl(ModuleDecl::ExportAll(..)) = item {
                return false;
            }

            item.is_module_decl()
        }) {
            return false;
        }
    }

    true
}
