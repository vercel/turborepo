use std::{collections::HashSet, future::Future, pin::Pin, task::Poll};

use turbo_tasks::Task;
use turbo_tasks_fs::{
    rebase, FileContent, FileContentRef, FileSystemPath, FileSystemPathRef, FileSystemRef,
};

use crate::utils::{all, race_pop};

#[turbo_tasks::value]
#[derive(PartialEq, Eq)]
pub struct CopyAllOptions {
    pub input_dir: FileSystemPathRef,
    pub output_dir: FileSystemPathRef,
}

#[turbo_tasks::function]
pub async fn copy_all(input: FileSystemPathRef, options: CopyAllOptionsRef) {
    let entry = module(input).await;
    let modules = all_modules(entry).await;

    all(modules
        .get()
        .modules
        .iter()
        .map(|module| copy_module(module.clone(), options.clone())))
    .await;
}

#[turbo_tasks::function]
async fn copy_module(module: ModuleRef, options: CopyAllOptionsRef) {
    let resource = &module.get().resource;
    let content = resource.clone().read().await;
    let options_value = options.get();
    let output = rebase(
        resource.clone(),
        options_value.input_dir.clone(),
        options_value.output_dir.clone(),
    )
    .await;
    output.write(content).await;
}

#[turbo_tasks::function]
async fn module(fs_path: FileSystemPathRef) -> ModuleRef {
    let source = fs_path.clone().read().await;
    let content = parse(source).await;
    Module {
        resource: fs_path,
        content,
    }
    .into()
}

#[turbo_tasks::value]
#[derive(PartialEq, Eq)]
struct Module {
    resource: FileSystemPathRef,
    content: ModuleContentRef,
}

#[turbo_tasks::value]
#[derive(PartialEq, Eq)]
struct ModuleContent {
    items: Vec<ModuleItemRef>,
}

#[turbo_tasks::value]
#[derive(PartialEq, Eq)]
enum ModuleItem {
    Comment(String),
    Reference(ModuleReferenceRef),
}

#[turbo_tasks::value]
#[derive(PartialEq, Eq)]
struct ModuleReference {
    request: String,
}

#[turbo_tasks::function]
async fn parse(content: FileContentRef) -> ModuleContentRef {
    match &*content.get() {
        FileContent::Content(bytes) => {
            let content = &*String::from_utf8_lossy(&bytes);
            let items: Vec<ModuleItemRef> = content
                .lines()
                .into_iter()
                .map(|line| {
                    if line.starts_with("#") {
                        ModuleItem::Comment(line[1..].to_string()).into()
                    } else {
                        ModuleItem::Reference(
                            ModuleReference {
                                request: line.to_string(),
                            }
                            .into(),
                        )
                        .into()
                    }
                })
                .collect();
            ModuleContent { items }.into()
        }
        FileContent::NotFound => {
            // report error
            ModuleContent { items: Vec::new() }.into()
        }
    }
}

#[turbo_tasks::value]
#[derive(PartialEq, Eq)]
struct ModulesSet {
    modules: HashSet<ModuleRef>,
}

#[turbo_tasks::function]
async fn all_modules(module: ModuleRef) -> ModulesSetRef {
    let mut modules = HashSet::new();
    let mut queue = vec![module];
    let mut futures_queue = Vec::new();
    loop {
        match queue.pop() {
            Some(module) => {
                modules.insert(module.clone());
                futures_queue.push(Box::pin(referenced_modules(module)));
            }
            None => match race_pop(&mut futures_queue).await {
                Some(modules_set) => {
                    for module in modules_set.get().modules.iter() {
                        queue.push(module.clone());
                    }
                }
                None => break,
            },
        }
    }
    assert!(futures_queue.is_empty());
    ModulesSet { modules }.into()
}

#[turbo_tasks::function]
async fn referenced_modules(origin: ModuleRef) -> ModulesSetRef {
    let mut modules = HashSet::new();
    for item in origin.get().content.get().items.iter() {
        match &*item.get() {
            ModuleItem::Comment(_) => {}
            ModuleItem::Reference(reference) => {
                let resolved = referenced_module(origin.clone(), reference.clone()).await;
                modules.insert(resolved);
            }
        }
    }
    ModulesSet { modules }.into()
}

#[turbo_tasks::function]
async fn referenced_module(origin: ModuleRef, reference: ModuleReferenceRef) -> ModuleRef {
    let resolved = resolve(origin.get().resource.clone(), reference.clone()).await;
    module(resolved).await
}

#[turbo_tasks::function]
async fn resolve(origin: FileSystemPathRef, reference: ModuleReferenceRef) -> FileSystemPathRef {
    let FileSystemPath { fs, path } = &*origin.get();
    let mut request = reference.get().request.to_string();
    let mut p = path.to_string();
    match p.rfind(|c| c == '/' || c == '\\') {
        Some(pos) => p.replace_range(pos.., ""),
        None => {}
    }
    loop {
        if request.starts_with("../") {
            request.replace_range(0..=2, "");
            match p.rfind(|c| c == '/' || c == '\\') {
                Some(pos) => p.replace_range(pos.., ""),
                None => {}
            }
        } else if request.starts_with("./") {
            request.replace_range(0..=1, "");
        } else {
            break;
        }
    }
    FileSystemPathRef::new(fs.clone(), p + "/" + &request)
}
