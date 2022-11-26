use std::{collections::VecDeque, io::Write as _};

use anyhow::Result;
use turbo_tasks::{primitives::StringVc, ValueToString};
use turbo_tasks_fs::rope::{Rope, RopeBuilder};
use turbopack_core::code_builder::{Code, CodeBuilder, CodeVc};
use turbopack_ecmascript::utils::stringify_module_id;

use super::{CssChunkItemVc, CssImport};
use crate::parse::{ParseResultSourceMap, ParseResultSourceMapVc};

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
        (0, "".to_string()),
    )];
    let mut result = ExpandImportsResult {
        external_imports: vec![],
        codes: vec![],
    };
    let mut indenter = Indent::default();

    while let Some((chunk_item, imports, (indent, close))) = stack.last_mut() {
        match imports.pop_front() {
            Some(CssImport::Internal(import, imported_chunk_item)) => {
                let (open, inner_indent, close) = import.await?.attributes.await?.print_block()?;

                let id = &*imported_chunk_item.to_string().await?;
                let mut code = CodeBuilderWithIndent::default();
                code.push_str(&format!("/* import({}) */\n", id), &indenter)?;
                code.push_str(&format!("{}\n", open), &indenter)?;
                result.codes.push(code.build().cell());

                let imported_content_vc = imported_chunk_item.content();
                let imported_content = &*imported_content_vc.await?;
                indenter.push(inner_indent);
                stack.push((
                    imported_chunk_item,
                    imported_content.imports.iter().cloned().collect(),
                    (inner_indent, close),
                ));
            }
            Some(CssImport::External(url_vc)) => {
                result.external_imports.push(url_vc);
            }
            None => {
                let mut code = CodeBuilderWithIndent::default();
                code.push_str(
                    &format!("/* {} */\n", stringify_module_id(&*chunk_item.id().await?)),
                    &indenter,
                )?;
                let content = chunk_item.content().await?;
                code.push_source(&content.inner_code, content.source_map, &indenter)
                    .await?;
                indenter.pop(*indent);
                code.push_str(&format!("\n{}\n", close), &indenter)?;
                result.codes.push(code.build().cell());
                stack.pop();
            }
        }
    }

    Ok(result.cell())
}

struct CodeBuilderWithIndent {
    code: CodeBuilder,
    needs_indent: bool,
}

impl Default for CodeBuilderWithIndent {
    fn default() -> Self {
        Self {
            code: CodeBuilder::default(),
            needs_indent: true,
        }
    }
}

#[derive(Default)]
struct Indent(String);

impl Indent {
    pub fn push(&mut self, n: usize) {
        self.0 += &" ".repeat(n);
    }

    pub fn pop(&mut self, n: usize) {
        self.0 = " ".repeat(self.0.len().saturating_sub(n));
    }

    pub fn to_str(&self) -> &str {
        &self.0
    }
}

impl CodeBuilderWithIndent {
    pub async fn push_source(
        &mut self,
        code: &Rope,
        map: Option<ParseResultSourceMapVc>,
        indent: &Indent,
    ) -> Result<()> {
        let indented = self.wrap_indent(&code.to_str()?, indent)?;
        let indented_map = if let Some(map) = map {
            Some(
                ParseResultSourceMap::with_indent(&*map.await?, indent.to_str().len() as u32)
                    .cell()
                    .as_generate_source_map(),
            )
        } else {
            None
        };
        self.code.push_source(&indented, indented_map);
        Ok(())
    }

    pub fn push_str(&mut self, code: &str, indent: &Indent) -> Result<()> {
        let indented = self.wrap_indent(code, indent)?;
        self.code.push_source(&indented, None);
        Ok(())
    }

    fn wrap_indent(&mut self, code: &str, indent: &Indent) -> Result<Rope> {
        let mut indented = RopeBuilder::default();
        for c in code.chars() {
            if c == '\n' {
                write!(indented, "\n")?;
                self.needs_indent = true;

                continue;
            }

            if self.needs_indent {
                write!(indented, "{}", indent.to_str())?;
                self.needs_indent = false;
            }

            write!(indented, "{}", c)?;
        }
        Ok(indented.build())
    }

    pub fn build(self) -> Code {
        self.code.build()
    }
}
