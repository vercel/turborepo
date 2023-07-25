use std::{io::Write, sync::Arc};

use anyhow::{Context, Result};
use swc_core::{
    base::{try_with_handler, Compiler},
    common::{
        BytePos, FileName, FilePathMapping, LineCol, Mark, SourceMap as SwcSourceMap, GLOBALS,
    },
    ecma::{self, ast::Program, codegen::Node},
};
use turbo_tasks::Vc;
use turbo_tasks_fs::FileSystemPath;
use turbopack_core::{
    code_builder::{Code, CodeBuilder},
    source_map::GenerateSourceMap,
};
use turbopack_ecmascript::ParseResultSourceMap;

#[turbo_tasks::function]
pub async fn minify(path: Vc<FileSystemPath>, code: Vc<Code>) -> Result<Vc<Code>> {
    let original_map = *code.generate_source_map().await?;
    let minified_code = perform_minify(path, code);

    let merged = match (original_map, *minified_code.generate_source_map().await?) {
        (Some(original_map), Some(minify_map)) => Some(Vc::upcast(original_map.trace(minify_map))),
        _ => None,
    };

    let mut builder = CodeBuilder::default();
    builder.push_source(minified_code.await?.source_code(), merged);
    let path = &*path.await?;
    let filename = path.file_name();
    write!(builder, "\n\n//# sourceMappingURL={}.map", filename)?;
    Ok(builder.build().cell())
}

#[turbo_tasks::function]
async fn perform_minify(path: Vc<FileSystemPath>, code_vc: Vc<Code>) -> Result<Vc<Code>> {
    let code = &*code_vc.await?;
    let cm = Arc::new(SwcSourceMap::new(FilePathMapping::empty()));
    let compiler = Arc::new(Compiler::new(cm.clone()));
    let fm = compiler.cm.new_source_file(
        FileName::Custom((*path.await?.path).to_string()),
        code.source_code().to_str()?.to_string(),
    );

    let lexer = ecma::parser::lexer::Lexer::new(
        ecma::parser::Syntax::default(),
        ecma::ast::EsVersion::latest(),
        ecma::parser::StringInput::from(&*fm),
        None,
    );
    let mut parser = ecma::parser::Parser::new_from(lexer);
    let program = try_with_handler(cm.clone(), Default::default(), |handler| {
        GLOBALS.set(&Default::default(), || {
            let program = parser.parse_program().unwrap();
            let unresolved_mark = Mark::new();
            let top_level_mark = Mark::new();

            Ok(compiler.run_transform(handler, false, || {
                swc_core::ecma::minifier::optimize(
                    program,
                    cm.clone(),
                    None,
                    None,
                    &ecma::minifier::option::MinifyOptions {
                        compress: Some(Default::default()),
                        ..Default::default()
                    },
                    &ecma::minifier::option::ExtraOptions {
                        top_level_mark,
                        unresolved_mark,
                    },
                )
            }))
        })
    })?;

    let (src, src_map_buf) = print_program(cm.clone(), program)?;

    let mut builder = CodeBuilder::default();
    builder.push_source(
        &src.into(),
        Some(*Box::new(Vc::upcast(
            ParseResultSourceMap::new(cm, src_map_buf).cell(),
        ))),
    );

    Ok(builder.build().cell())
}

// From https://github.com/swc-project/swc/blob/11efd4e7c5e8081f8af141099d3459c3534c1e1d/crates/swc/src/lib.rs#L523-L560
fn print_program(
    cm: Arc<SwcSourceMap>,
    program: Program,
) -> Result<(String, Vec<(BytePos, LineCol)>)> {
    let mut src_map_buf = vec![];

    let src = {
        let mut buf = vec![];
        {
            let wr = Box::new(swc_core::ecma::codegen::text_writer::omit_trailing_semi(
                Box::new(swc_core::ecma::codegen::text_writer::JsWriter::new(
                    cm.clone(),
                    "\n",
                    &mut buf,
                    Some(&mut src_map_buf),
                )),
            )) as Box<dyn swc_core::ecma::codegen::text_writer::WriteJs>;

            let mut emitter = swc_core::ecma::codegen::Emitter {
                cfg: swc_core::ecma::codegen::Config {
                    minify: true,
                    ..Default::default()
                },
                comments: None,
                cm: cm.clone(),
                wr,
            };

            program
                .emit_with(&mut emitter)
                .context("failed to emit module")?;
        }
        // Invalid utf8 is valid in javascript world.
        String::from_utf8(buf).expect("invalid utf8 character detected")
    };

    Ok((src, src_map_buf))
}
