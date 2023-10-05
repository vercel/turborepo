use swc_core::ecma::ast::{Module, NamedExport};

pub struct BarrelOptimizer {}

/// The type of an ECMAScript file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Normal,
    /// File with only re-exports.
    Barrel,
    Full,
}

impl FileType {
    pub fn detect(module: &Module) -> Self {
        if module.body.iter().all(|i| match i {
            swc_core::ecma::ast::ModuleItem::ModuleDecl(s) => match s {
                swc_core::ecma::ast::ModuleDecl::Import(_)
                | swc_core::ecma::ast::ModuleDecl::ExportNamed(NamedExport {
                    src: Some(..), ..
                })
                | swc_core::ecma::ast::ModuleDecl::ExportAll(_) => true,
                _ => false,
            },
            swc_core::ecma::ast::ModuleItem::Stmt(_) => false,
        }) {
            return Self::Barrel;
        }

        Self::Full
    }
}
