use std::{collections::VecDeque, io::Write};

use anyhow::Result;
use turbo_tasks::{primitives::StringVc, ValueToString};
use turbopack_core::code_builder::{CodeBuilder, CodeVc};

use super::{CssChunkItemVc, CssImport};

#[turbo_tasks::value]
#[derive(Default)]
pub struct ExpandImportsResult {
    pub external_imports: Vec<StringVc>,
    pub codes: Vec<CodeVc>,
}

#[turbo_tasks::function]
pub async fn expand_imports(chunk_item: CssChunkItemVc) -> Result<ExpandImportsResultVc> {
    let content = chunk_item.content().await?;
    let mut stack = vec![(
        chunk_item,
        content.imports.iter().cloned().collect::<VecDeque<_>>(),
        "".to_string(),
    )];
    let mut result = ExpandImportsResult {
        external_imports: vec![],
        codes: vec![],
    };

    while let Some((chunk_item, imports, close)) = stack.last_mut() {
        match imports.pop_front() {
            Some(CssImport::Internal(import, imported_chunk_item)) => {
                let (open, close) = import.await?.attributes.await?.print_block()?;

                let id = &*imported_chunk_item.to_string().await?;
                let mut code = CodeBuilder::default();
                writeln!(code, "/* import({}) */", id)?;
                writeln!(code, "{}", open)?;
                result.codes.push(code.build().cell());

                let imported_content_vc = imported_chunk_item.content();
                let imported_content = &*imported_content_vc.await?;
                stack.push((
                    imported_chunk_item,
                    imported_content.imports.iter().cloned().collect(),
                    close,
                ));
            }
            Some(CssImport::External(url_vc)) => {
                result.external_imports.push(url_vc);
            }
            None => {
                let mut code = CodeBuilder::default();
                let id = &*chunk_item.to_string().await?;
                writeln!(code, "/* {} */", id)?;

                let content = chunk_item.content().await?;
                code.push_source(
                    &content.inner_code,
                    content.source_map.map(|sm| sm.as_generate_source_map()),
                );
                writeln!(code, "\n{}", close)?;

                result.codes.push(code.build().cell());
                stack.pop();
            }
        }
    }

    Ok(result.cell())
}
