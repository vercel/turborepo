use swc_core::ecma::{ast::*, visit::Visit};

#[derive(Default)]
pub(super) struct TopLevelAwaitVisitor {
    pub(super) has_top_level_await: bool,
}

impl Visit for TopLevelAwaitVisitor {
    fn visit_await_expr(&mut self, _: &AwaitExpr) {
        self.has_top_level_await = true;
    }

    // prevent non top level items from visiting their children
    fn visit_arrow_expr(&mut self, _: &ArrowExpr) {}
    fn visit_class(&mut self, _: &Class) {}
    fn visit_function(&mut self, _: &Function) {}
}
