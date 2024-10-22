use crate::common::check_query;

mod common;

#[cfg(not(windows))]
#[test]
fn test_double_symlink() -> Result<(), anyhow::Error> {
    check_query(
        "oxc_repro",
        vec![
            "query {
                 file(path: \"./index.js\") {
                    path
                    dependencies {
                      files { items { path } }
                      errors { items { message import } }
                    }
                 }
              }",
        ],
    )?;
    Ok(())
}

fn test_trace() -> Result<(), anyhow::Error> {
    check_query(
        "turbo_trace",
        vec![
            "query { file(path: \"main.ts\") { path } }",
            "query { file(path: \"main.ts\") { path, dependencies { files { items { path } } } } }",
            "query { file(path: \"button.tsx\") { path, dependencies { files { items { path } } } \
             } }",
            "query { file(path: \"circular.ts\") { path dependencies { files { items { path } } } \
             } }",
            "query { file(path: \"invalid.ts\") { path dependencies { files { items { path } } \
             errors { items { message } } } } }",
            "query { file(path: \"main.ts\") { path ast } }",
            "query { file(path: \"main.ts\") { path dependencies(depth: 1) { files { items { path \
             } } } } }",
        ],
    )?;

    Ok(())
}
