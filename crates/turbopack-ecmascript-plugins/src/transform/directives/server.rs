use anyhow::Result;
use async_trait::async_trait;
use swc_core::ecma::{ast::Program, transforms::base::resolver, visit::VisitMutWith};
use turbo_tasks::Vc;
use turbopack_ecmascript::{CustomTransformer, TransformContext};

use super::{is_server_module, server_to_client_proxy::create_proxy_module};

#[derive(Debug)]
pub struct ServerDirectiveTransformer {
    // ServerDirective is not implemented yet and always reports an issue.
    // We don't have to pass a valid transition name yet, but the API is prepared.
    #[allow(unused)]
    transition_name: Vc<String>,
}

impl ServerDirectiveTransformer {
    pub fn new(transition_name: Vc<String>) -> Self {
        Self { transition_name }
    }
}

#[async_trait]
impl CustomTransformer for ServerDirectiveTransformer {
    async fn transform(&self, program: &mut Program, ctx: &TransformContext<'_>) -> Result<()> {
        if is_server_module(program) {
            let transition_name = &*self.transition_name.await?;
            // *program = create_proxy_module(transition_name, &format!("./{}",
            // ctx.file_name_str)); program.visit_mut_with(&mut
            // resolver( ctx.unresolved_mark,
            // ctx.top_level_mark,
            // false,
            // ));
        }

        Ok(())
    }
}
