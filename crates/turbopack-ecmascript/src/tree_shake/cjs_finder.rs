use swc_core::ecma::ast::*;

use crate::TURBOPACK_HELPER;

pub fn should_skip_tree_shaking(m: &Program) -> bool {
    if let Program::Module(m) = m {
        match m.body.iter().any(|item| {
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

            match item {
                // We don't have logic to tree shake export * from
                ModuleItem::ModuleDecl(ModuleDecl::ExportAll(..)) => return false,

                // Tree shaking has a bug related to ModuleExportName::Str
                ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                    for s in import.specifiers.iter() {
                        if let ImportSpecifier::Named(is) = s {
                            if matches!(is.imported, Some(ModuleExportName::Str(..))) {
                                return false;
                            }
                        }
                    }
                }

                // Tree shaking has a bug related to ModuleExportName::Str
                ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(NamedExport {
                    src: Some(..),
                    specifiers,
                    ..
                })) => {
                    for s in specifiers {
                        if let ExportSpecifier::Named(es) = s {
                            if matches!(es.orig, ModuleExportName::Str(..))
                                || matches!(es.exported, Some(ModuleExportName::Str(..)))
                            {
                                return false;
                            }
                        }
                    }
                }

                _ => (),
            }

            item.is_module_decl()
        }) {
            true => return false,
            false => (),
        }
    }

    true
}
