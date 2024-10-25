mod common;

#[cfg(not(windows))]
#[test]
fn test_double_symlink() -> Result<(), anyhow::Error> {
    check_json!(
        "oxc_repro",
        "npm@10.5.0",
        "query",
        "get_dependencies" => "query {
                 file(path: \"./index.js\") {
                    path
                    dependencies {
                      files { items { path } }
                      errors { items { message import } }
                    }
                 }
              }",
    );
    Ok(())
}

#[test]
fn test_ast() -> Result<(), anyhow::Error> {
    // Separate because the `\\` -> `/` filter isn't compatible with ast
    check_json!(
        "turbo_trace",
        "npm@10.5.0",
        "query",
        "get `main.ts` with ast" => "query { file(path: \"main.ts\") { path ast } }",
    );

    Ok(())
}

#[test]
fn test_trace() -> Result<(), anyhow::Error> {
    insta::with_settings!({ filters => vec![(r"\\\\", "/")]}, {
        check_json!(
            "turbo_trace",
            "npm@10.5.0",
            "query",
            "get `main.ts`" => "query { file(path: \"main.ts\") { path } }",
            "get `main.ts` with dependencies" => "query { file(path: \"main.ts\") { path, dependencies { files { items { path } } } } }",
            "get `button.tsx` with dependencies" => "query { file(path: \"button.tsx\") { path, dependencies { files { items { path } } } } }",
            "get `circular.ts` with dependencies" => "query { file(path: \"circular.ts\") { path dependencies { files { items { path } } } } }",
            "get `invalid.ts` with dependencies" => "query { file(path: \"invalid.ts\") { path dependencies { files { items { path } } errors { items { message } } } } }",
            "get `main.ts` with depth = 0" => "query { file(path: \"main.ts\") { path dependencies(depth: 1) { files { items { path } } } } }",
            "get `with_prefix.ts` with dependencies" => "query { file(path: \"with_prefix.ts\") { path dependencies { files { items { path } } } } }",
        );

        Ok(())
    })
}

#[test]
fn test_trace_on_monorepo() -> Result<(), anyhow::Error> {
    insta::with_settings!({ filters => vec![(r"\\\\", "/")]}, {
        check_json!(
            "turbo_trace_monorepo",
            "npm@10.5.0",
            "query",
            "get `apps/my-app/index.ts` with dependencies" => "query { file(path: \"apps/my-app/index.ts\") { path dependencies { files { items { path } } errors { items { message } } } } }",
            "get `packages/utils/index` with dependents" => "query { file(path: \"packages/utils/index.ts\") { path dependents { files { items { path } } errors { items { message } } } } }",
        );

        Ok(())
    })
}

#[test]
fn test_reverse_trace() -> Result<(), anyhow::Error> {
    check_json!(
        "turbo_trace",
        "npm@10.5.0",
        "query",
        "get `button.tsx` with dependents" => "query { file(path: \"button.tsx\") { path dependents { files { items { path } } } } }",
    );

    Ok(())
}
