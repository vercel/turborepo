use lightningcss::{stylesheet::StyleSheet, visitor::Visitor};
use turbo_tasks::Vc;
use turbopack_core::chunk::ChunkingContext;

use crate::{chunk::CssImport, references::AstParentKind};

/// impl of code generation inferred from a ModuleReference.
/// This is rust only and can't be implemented by non-rust plugins.
#[turbo_tasks::value(
    shared,
    serialization = "none",
    eq = "manual",
    into = "new",
    cell = "new"
)]
pub struct CodeGeneration {
    /// ast nodes matching the span will be visitor by the visitor
    #[turbo_tasks(debug_ignore, trace_ignore)]
    pub visitors: Vec<(Vec<AstParentKind>, Box<dyn VisitorFactory>)>,
    #[turbo_tasks(debug_ignore, trace_ignore)]
    pub imports: Vec<CssImport>,
}

pub trait VisitorFactory: Send + Sync {
    fn create<'a>(&'a self) -> VisitorLike<'a>;
}

pub struct VisitorLike<'a> {
    op: Box<dyn 'a + FnOnce(&mut StyleSheet<'static, 'static>) + Send + Sync>,
}

#[turbo_tasks::value_trait]
pub trait CodeGenerateable {
    fn code_generation(
        self: Vc<Self>,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Vc<CodeGeneration>;
}

#[turbo_tasks::value(transparent)]
pub struct CodeGenerateables(Vec<Vc<Box<dyn CodeGenerateable>>>);

pub fn path_to(
    path: &[AstParentKind],
    f: impl FnMut(&AstParentKind) -> bool,
) -> Vec<AstParentKind> {
    if let Some(pos) = path.iter().rev().position(f) {
        let index = path.len() - pos - 1;
        path[..index].to_vec()
    } else {
        path.to_vec()
    }
}

#[macro_export]
macro_rules! create_visitor {
    (exact $ast_path:expr, $name:ident($arg:ident: &mut $ty:ident) $b:block) => {
        $crate::create_visitor!(__ $ast_path.to_vec(), $name($arg: &mut $ty) $b)
    };
    ($ast_path:expr, $name:ident($arg:ident: &mut $ty:ident) $b:block) => {
        $crate::create_visitor!(__ $crate::code_gen::path_to(&$ast_path, |n| {
            matches!(n, $crate::references::AstParentKind::$ty(_))
        }), $name($arg: &mut $ty) $b)
    };
    (__ $ast_path:expr, $name:ident($arg:ident: &mut $ty:ident) $b:block) => {{
        struct Visitor<T: Fn(&mut $ty) + Send + Sync> {
            $name: T,
        }

        impl<T: Fn(&mut $ty) + Send + Sync> $crate::code_gen::VisitorFactory
            for Visitor<T>
        {
            fn create<'a>(&'a self) -> $crate::code_gen::VisitorLike<'a> {
                use lightningcss::visitor::Visit;
                $crate::code_gen::VisitorLike {
                    op: Box::new(move |s: &mut lightningcss::stylesheet::StyleSheet| {
                        s.visit(&mut self);
                    }),
                }
            }
        }

        impl<'a, T: Fn(&mut $ty) + Send + Sync> lightningcss::visitor::Visitor<'_>
            for &'a Visitor<T>
        {
            fn $name(&mut self, $arg: &mut $ty) {
                (self.$name)($arg);
            }
        }

        (
            $ast_path,
            Box::new(Box::new(Visitor {
                $name: move |$arg: &mut $ty| $b,
            })) as Box<dyn $crate::code_gen::VisitorFactory>,
        )
    }};
    (visit_mut_stylesheet($arg:ident: &mut Stylesheet) $b:block) => {{
        struct Visitor<T: Fn(&mut Stylesheet) + Send + Sync> {
            visit_mut_stylesheet: T,
        }

        impl<T: Fn(&mut Stylesheet) + Send + Sync> $crate::code_gen::VisitorFactory
            for Box<Visitor<T>>
        {
            fn create<'a>(&'a self) -> Box<dyn VisitMut + Send + Sync + 'a> {
                Box::new(&**self)
            }
        }

        impl<'a, T: Fn(&mut Stylesheet) + Send + Sync> lightningcss::visitor::Visit
            for &'a Visitor<T>
        {
            fn visit_mut_stylesheet(&mut self, $arg: &mut Stylesheet) {
                (self.visit_mut_stylesheet)($arg);
            }
        }

        (
            Vec::new(),
            Box::new(Box::new(Visitor {
                visit_mut_stylesheet: move |$arg: &mut Stylesheet| $b,
            })) as Box<dyn $crate::code_gen::VisitorFactory>,
        )
    }};
}
