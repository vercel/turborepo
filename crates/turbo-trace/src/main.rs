use std::{collections::HashSet, env, fs, sync::Arc};

use camino::Utf8PathBuf;
use oxc_resolver::{ResolveError, ResolveOptions, Resolver};
use swc_common::{comments::SingleThreadedComments, FileName, SourceMap};
use swc_ecma_ast::{Decl, EsVersion, ModuleDecl, Stmt};
use swc_ecma_parser::{lexer::Lexer, Capturing, EsSyntax, Parser, StringInput, Syntax, TsSyntax};
use swc_ecma_visit::{Visit, VisitWith};
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;

struct Tracer {
    cwd: AbsoluteSystemPathBuf,
    files: Vec<AbsoluteSystemPathBuf>,
    seen: HashSet<AbsoluteSystemPathBuf>,
}

#[derive(Debug, Error)]
enum Error {
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

impl Tracer {
    fn new(entry: Utf8PathBuf) -> Result<Self, Error> {
        let cwd = AbsoluteSystemPathBuf::cwd()?;
        let absolute_entry = AbsoluteSystemPathBuf::from_unknown(&cwd, entry);
        let mut files = vec![absolute_entry];
        let mut seen = HashSet::new();

        Ok(Self { cwd, files, seen })
    }
    fn trace(mut self) -> Vec<String> {
        let options = ResolveOptions::default().with_builtin_modules(true);
        let resolver = Resolver::new(options);
        let mut errors = vec![];

        while let Some(file_path) = self.files.pop() {
            if matches!(file_path.extension(), Some("json") | Some("css")) {
                continue;
            }
            println!("{}", file_path);

            if self.seen.contains(&file_path) {
                continue;
            }

            self.seen.insert(file_path.clone());

            // Read the file content
            let file_content = match fs::read_to_string(&file_path) {
                Ok(content) => content,
                Err(err) => {
                    eprintln!("Error reading file {}: {}", file_path, err);
                    std::process::exit(1);
                }
            };

            // Setup SWC
            let cm = Arc::new(SourceMap::default());
            let comments = SingleThreadedComments::default();

            let fm = cm.new_source_file(
                FileName::Custom(file_path.to_string()).into(),
                file_content.into(),
            );

            let syntax =
                if file_path.extension() == Some("ts") || file_path.extension() == Some("tsx") {
                    Syntax::Typescript(TsSyntax {
                        tsx: file_path.extension() == Some("tsx"),
                        decorators: true,
                        ..Default::default()
                    })
                } else {
                    Syntax::Es(EsSyntax {
                        jsx: file_path.ends_with(".jsx"),
                        ..Default::default()
                    })
                };

            let lexer = Lexer::new(
                syntax,
                EsVersion::EsNext,
                StringInput::from(&*fm),
                Some(&comments),
            );

            let mut parser = Parser::new_from(Capturing::new(lexer));

            // Parse the file as a module
            let module = match parser.parse_module() {
                Ok(module) => module,
                Err(err) => {
                    errors.push(format!("failed to parse module {}: {:?}", file_path, err));
                    continue;
                }
            };

            // Visit the AST and find imports
            let mut finder = ImportFinder { imports: vec![] };
            module.visit_with(&mut finder);

            // Print found imports/requires
            for import in finder.imports {
                let file_dir = file_path.parent().unwrap_or(&self.cwd);
                match resolver.resolve(&file_dir, &import) {
                    Ok(resolved) => match resolved.into_path_buf().try_into() {
                        Ok(path) => self.files.push(path),
                        Err(err) => {
                            errors.push(err.to_string());
                        }
                    },
                    Err(ResolveError::Builtin(_)) => {}
                    Err(err) => {
                        errors.push(err.to_string());
                    }
                }
            }
        }

        errors
    }
}

struct ImportFinder {
    imports: Vec<String>,
}

impl Visit for ImportFinder {
    fn visit_module_decl(&mut self, decl: &ModuleDecl) {
        if let ModuleDecl::Import(import) = decl {
            self.imports.push(import.src.value.to_string());
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        if let Stmt::Decl(Decl::Var(var_decl)) = stmt {
            for decl in &var_decl.decls {
                if let Some(init) = &decl.init {
                    if let swc_ecma_ast::Expr::Call(call_expr) = &**init {
                        if let swc_ecma_ast::Callee::Expr(expr) = &call_expr.callee {
                            if let swc_ecma_ast::Expr::Ident(ident) = &**expr {
                                if ident.sym == *"require" {
                                    if let Some(arg) = call_expr.args.first() {
                                        if let swc_ecma_ast::Expr::Lit(swc_ecma_ast::Lit::Str(
                                            lit_str,
                                        )) = &*arg.expr
                                        {
                                            self.imports.push(lit_str.value.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        stmt.visit_children_with(self);
    }
}

fn main() -> Result<(), Error> {
    // Get the file path from command line arguments
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} --file <path_to_js_or_ts_file>", args[0]);
        std::process::exit(1);
    }

    let file_flag = &args[1];
    if file_flag != "--file" {
        eprintln!("Usage: {} --file <path_to_js_or_ts_file>", args[0]);
        std::process::exit(1);
    }

    let file_path = &args[2];
    let tracer = Tracer::new(file_path.into())?;

    let errors = tracer.trace();

    if !errors.is_empty() {
        for error in errors {
            eprintln!("{}", error);
        }
        std::process::exit(1);
    }

    Ok(())
}
