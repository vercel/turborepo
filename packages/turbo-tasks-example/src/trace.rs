use turbo_tasks_fs::{
    rebase, FileContent, FileContentRef, FileSystemPath, FileSystemPathRef, FileSystemRef,
};

#[turbo_tasks::value]
#[derive(PartialEq, Eq)]
pub struct CopyAllOptions {
    pub input_dir: FileSystemPathRef,
    pub output_dir: FileSystemPathRef,
}

#[turbo_tasks::function]
pub async fn copy_all(input: FileSystemPathRef, options: CopyAllOptionsRef) {
    println!("copy_all {}", input.get().path);
    let options_value = options.get();
    let content = input.clone().read().await;
    let output = rebase(
        input.clone(),
        options_value.input_dir.clone(),
        options_value.output_dir.clone(),
    )
    .await;
    let write = output.write(content.clone());

    let module = parse(content).await;
    for future in module
        .get()
        .items
        .iter()
        .map(|item| process_module_item(input.clone(), item.clone(), options.clone()))
        .collect::<Vec<_>>()
        .into_iter()
    {
        future.await;
    }
    write.await;
}

#[turbo_tasks::function]
async fn process_module_item(
    origin: FileSystemPathRef,
    item: ModuleItemRef,
    options: CopyAllOptionsRef,
) {
    match &*item.get() {
        ModuleItem::Comment(_) => {}
        ModuleItem::Reference(reference) => {
            let resolved = resolve(origin.clone(), reference.clone()).await;
            copy_all(resolved, options.clone()).await
        }
    }
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

#[turbo_tasks::value]
#[derive(PartialEq, Eq)]
struct Module {
    resource: FileSystemRef,
    content: ModuleContent,
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
