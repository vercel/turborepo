use turbo_tasks_fs::{embed_directory, FileContentVc};

#[turbo_tasks::function]
pub(crate) fn embed_file(path: &str) -> FileContentVc {
    embed_directory!("next", "$CARGO_MANIFEST_DIR/js/src")()
        .root()
        .join(path)
        .read()
}
