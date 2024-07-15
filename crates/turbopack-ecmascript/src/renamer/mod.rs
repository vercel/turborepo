use anyhow::Result;
use swc_core::ecma::{
    ast::{ModuleItem, Program},
    transforms::base::hygiene::hygiene,
    visit::VisitMutWith,
};
use turbo_tasks::{RcStr, Vc};

use crate::{analyzer::graph::EvalContext, parse::ParseResult};

/// Rename `modules` so those modules has no conflict regardless of the way we
/// use for merging.
///
/// The returned modules are guaranteed to have same order as input.
///
/// Designed after the renamer of esbuild.
///
/// This renamer renames non-top-level identifiers in parallel, and top-level
/// identifiers in series.
#[turbo_tasks::function]
pub async fn rename_modules(modules: Vc<Vec<Vc<ParseResult>>>) -> Result<Vc<Vec<Vc<ParseResult>>>> {
    let modules = modules.await?;
    let mut buf = Vec::with_capacity(modules.len());

    for &module in modules.iter() {
        buf.push(rename_module(module).resolve().await?);
    }

    // Collect top level identifiers

    Ok(Vc::cell(buf))
}

#[turbo_tasks::function]
async fn rename_module(module: Vc<ParseResult>) -> Result<Vc<ParseResult>> {
    match &*module.await? {
        ParseResult::Ok {
            program,
            comments,
            eval_context,
            globals,
            source_map,
        } => {
            let mut program = program.clone();

            program.visit_mut_with(&mut hygiene());

            let eval_context = EvalContext::new(
                &program,
                eval_context.unresolved_mark,
                eval_context.top_level_mark,
                false,
                None,
            );

            Ok(ParseResult::Ok {
                program,
                comments: comments.clone(),
                eval_context,
                globals: globals.clone(),
                source_map: source_map.clone(),
            }
            .cell())
        }
        _ => Ok(module),
    }
}

#[turbo_tasks::function]
async fn collect_top_level_identifiers(module: Vc<ParseResult>) -> Result<Vc<Vec<RcStr>>> {
    match &*module.await? {
        ParseResult::Ok { program, .. } => {
            let ids = find_top_level_ids(program);

            Ok(Vc::cell(ids))
        }

        _ => Ok(Vc::cell(Vec::new())),
    }
}

fn find_top_level_ids(program: &Program) -> Vec<RcStr> {
    let Program::Module(program) = program else {
        // We can't merge scrips
        return vec![];
    };
    let mut ids = vec![];

    for item in &program.body {
        match item {
            ModuleItem::ModuleDecl(item) => {}
            ModuleItem::Stmt(item) => {}
        }
    }

    ids
}
