use anyhow::Result;
use turbo_tasks::{debug::ValueDebugFormat, CompletionVc};
use turbo_tasks_testing::{register, run};

register!();

#[turbo_tasks::function]
async fn ignored_indexes() -> Result<CompletionVc> {
    #[derive(Clone, ValueDebugFormat)]
    struct IgnoredIndexes(
        #[turbo_tasks(debug_ignore)] i32,
        i32,
        #[turbo_tasks(debug_ignore)] i32,
    );

    let input = IgnoredIndexes(-0, 1, -2);
    let debug = input.value_debug_format(usize::MAX).try_to_string().await?;
    assert!(!debug.contains("0"));
    assert!(debug.contains("1"));
    assert!(!debug.contains("2"));

    Ok(CompletionVc::immutable())
}

#[tokio::test]
async fn tests() {
    run! {
        ignored_indexes().await?;
    }
}
