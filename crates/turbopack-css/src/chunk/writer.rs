use std::{collections::VecDeque, fmt::Write, io::Write as _};

use anyhow::Result;
use turbo_tasks::{primitives::StringVc, ValueToString};
use turbo_tasks_fs::rope::RopeVc;
use turbopack_core::code_builder::{Code, CodeBuilder, CodeVc};

use super::CssImport;
use crate::{parse::ParseResultSourceMapVc, CssChunkItemContentVc};

#[turbo_tasks::value]
#[derive(Default)]
pub struct ExpandImportsResult {
    pub external_imports: Vec<StringVc>,
    pub codes: Vec<CodeVc>,
}

#[turbo_tasks::function]
pub async fn expand_imports(content_vc: CssChunkItemContentVc) -> Result<ExpandImportsResultVc> {
    let content = &*content_vc.await?;
    let mut stack = vec![(
        content_vc,
        content.imports.iter().cloned().collect::<VecDeque<_>>(),
        (0, "".to_string()),
    )];
    let mut result = ExpandImportsResult {
        external_imports: vec![],
        codes: vec![],
    };

    while let Some((content_vc, imports, (indent, close))) = stack.last_mut() {
        match imports.pop_front() {
            Some(CssImport::Internal(import, imported_chunk_item)) => {
                let (open, inner_indent, close) = import.await?.attributes.await?.print_block()?;

                let id = &*imported_chunk_item.to_string().await?;
                let mut code = CodeBuilder::default();
                writeln!(code, "/* import({}) */", id);
                writeln!(code, "{}", open);
                result.codes.push(code.build().cell());

                let imported_content_vc = imported_chunk_item.content();
                let imported_content = &*imported_content_vc.await?;
                stack.push((
                    imported_content_vc,
                    imported_content.imports.iter().cloned().collect(),
                    (inner_indent, close),
                ));
            }
            Some(CssImport::External(url_vc)) => {
                result.external_imports.push(url_vc);
            }
            None => {
                let content = &*(*content_vc).await?;
                let mut code = CodeBuilder::default();
                writeln!(code, "{}", content.inner_code)?;
                writeln!(code, "{}", close)?;
                result.codes.push(code.build().cell());
                stack.pop();
            }
        }
    }

    Ok(result.cell())
}

pub struct WriterWithIndent<T: Write> {
    writer: T,
    indent_str: String,
    needs_indent: bool,
}

impl<T: Write> WriterWithIndent<T> {
    pub fn new(buffer: T) -> Self {
        Self {
            writer: buffer,
            indent_str: "".to_string(),
            needs_indent: true,
        }
    }

    pub fn push_indent(&mut self, indent: usize) -> std::fmt::Result {
        self.indent_str += &" ".repeat(indent);

        Ok(())
    }

    pub fn pop_indent(&mut self, indent: usize) -> std::fmt::Result {
        self.indent_str = " ".repeat(self.indent_str.len().saturating_sub(indent));

        Ok(())
    }
}

impl<T: Write> Write for WriterWithIndent<T> {
    #[inline]
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        for c in s.chars() {
            self.write_char(c)?;
        }

        Ok(())
    }

    #[inline]
    fn write_char(&mut self, c: char) -> std::fmt::Result {
        if c == '\n' {
            self.writer.write_char('\n')?;
            self.needs_indent = true;

            return Ok(());
        }

        if self.needs_indent {
            self.writer.write_str(&self.indent_str)?;
            self.needs_indent = false;
        }

        self.writer.write_char(c)
    }
}
