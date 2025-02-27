mod common;

#[test]
fn test_query() -> Result<(), anyhow::Error> {
    check_json_output!(
        "basic_monorepo",
        "npm@10.5.0",
        "query",
        "get packages" => ["query { packages { items { name } } }"],
        "get packages with equals filter" => ["query { packages(filter: { equal: { field: NAME, value: \"my-app\" } }) { items { name } } }"],
        "get package that doesn't exist" => ["query { package(name: \"doesnotexist\") { path } }"],
        "get packages with less than 1 dependents" => ["query { packages(filter: {lessThan: {field: DIRECT_DEPENDENT_COUNT, value: 1}}) { items { name directDependents { length } } } }"],
        "get packages with more than 0 dependents" => ["query { packages(filter: {greaterThan: {field: DIRECT_DEPENDENT_COUNT, value: 0}}) { items { name directDependents { length } } } }"],
        "get packages that have a task named `build`" => ["query { packages(filter: {has: { field: TASK_NAME, value: \"build\" }}) { items { name } } }"],
        "get packages that have a task named `build` or `dev`" => ["query { packages(filter: {or: [{ has: { field: TASK_NAME, value: \"build\" } }, { has: { field: TASK_NAME, value: \"dev\" } }] }) { items { name } } }"],
        "get dependents of `util`" => ["query { packages(filter: {equal: { field: NAME, value: \"util\" } }) { items { directDependents { items { name } } } } }"],
        "get dependencies of `my-app`" => ["query { packages(filter: {equal: { field: NAME, value: \"my-app\" } }) { items { directDependencies { items { name } } } } }"],
        "get the indirect dependencies of `my-app`" => ["query { packages(filter: {equal: { field: NAME, value: \"my-app\" } }) { items { indirectDependencies { items { name } } } } }"],
        "get all dependencies of `my-app`" => ["query { packages(filter: {equal: { field: NAME, value: \"my-app\" } }) { items { allDependencies { items { name } } } } }"],
        "get package graph" => ["query { packageGraph { nodes { items { name } } edges { items { source target } } } }"],
        "get schema" => ["--schema"],
    );

    Ok(())
}

#[cfg(not(windows))]
#[test]
fn test_double_symlink() -> Result<(), anyhow::Error> {
    check_json_output!(
        "oxc_repro",
        "npm@10.5.0",
        "query",
        "get_dependencies" => ["query {
                 file(path: \"./index.js\") {
                    path
                    dependencies {
                      files { items { path } }
                      errors { items { message import } }
                    }
                 }
              }"],
    );
    Ok(())
}

#[test]
fn test_ast() -> Result<(), anyhow::Error> {
    // Separate because the `\\` -> `/` filter isn't compatible with ast
    check_json_output!(
        "turbo_trace",
        "npm@10.5.0",
        "query",
        "get `main.ts` with ast" => ["query { file(path: \"main.ts\") { path ast } }"],
    );

    Ok(())
}

#[test]
fn test_trace() -> Result<(), anyhow::Error> {
    insta::with_settings!({ filters => vec![(r"\\\\", "/")]}, {
        check_json_output!(
            "turbo_trace",
            "npm@10.5.0",
            "query",
            "get `main.ts`" => ["query { file(path: \"main.ts\") { path } }"],
            "get `main.ts` with dependencies" => ["query { file(path: \"main.ts\") { path, dependencies { files { items { path } } } } }"],
            "get `button.tsx` with dependencies" => ["query { file(path: \"button.tsx\") { path, dependencies { files { items { path } } } } }"],
            "get `circular.ts` with dependencies" => ["query { file(path: \"circular.ts\") { path dependencies { files { items { path } } } } }"],
            "get `invalid.ts` with dependencies" => ["query { file(path: \"invalid.ts\") { path dependencies { files { items { path } } errors { items { import } } } } }"],
            "get `main.ts` with depth = 0" => ["query { file(path: \"main.ts\") { path dependencies(depth: 1) { files { items { path } } } } }"],
            "get `with_prefix.ts` with dependencies" => ["query { file(path: \"with_prefix.ts\") { path dependencies { files { items { path } } } } }"],
            "get `import_value_and_type.ts` with all dependencies" => ["query { file(path: \"import_value_and_type.ts\") { path dependencies(importType: ALL) { files { items { path } } } } }"],
            "get `import_value_and_type.ts` with type dependencies" => ["query { file(path: \"import_value_and_type.ts\") { path dependencies(importType: TYPES) { files { items { path } } } } }"],
            "get `import_value_and_type.ts` with value dependencies" => ["query { file(path: \"import_value_and_type.ts\") { path dependencies(importType: VALUES) { files { items { path } } } } }"],
            "get `incorrect_extension.mjs` with dependencies" =>  ["query { file(path: \"incorrect_extension.mjs\") { path dependencies(depth: 1) { files { items { path } } } } }"],
            "get `export_all.js` with dependencies" => ["query { file(path: \"export_all.js\") { path dependencies { files { items { path } } } } }"],
            "get `export_named.js` with dependencies" => ["query { file(path: \"export_named.js\") { path dependencies { files { items { path } } } } }"],
        );

        Ok(())
    })
}

#[test]
fn test_trace_on_monorepo() -> Result<(), anyhow::Error> {
    insta::with_settings!({ filters => vec![(r"\\\\", "/")]}, {
        check_json_output!(
            "turbo_trace_monorepo",
            "npm@10.5.0",
            "query",
            "get `apps/my-app/index.ts` with dependencies" => ["query { file(path: \"apps/my-app/index.ts\") { path dependencies { files { items { path } } errors { items { message } } } } }"],
            "get `packages/utils/index.ts` with dependents" => ["query { file(path: \"packages/utils/index.ts\") { path dependents { files { items { path } } errors { items { message } } } } }"],
            "get `packages/another/index.js` with dependents" => ["query { file(path: \"packages/another/index.jsx\") { path dependents { files { items { path } } errors { items { message } } } } }"],
        );

        Ok(())
    })
}

#[test]
fn test_reverse_trace() -> Result<(), anyhow::Error> {
    check_json_output!(
        "turbo_trace",
        "npm@10.5.0",
        "query",
        "get `button.tsx` with dependents" => ["query { file(path: \"button.tsx\") { path dependents { files { items { path } } } } }"],
        "get `link.tsx` with all dependents" => ["query { file(path: \"link.tsx\") { path dependents(importType: ALL) { files { items { path } } } } }"],
        "get `link.tsx` with type dependents" => ["query { file(path: \"link.tsx\") { path dependents(importType: TYPES) { files { items { path } } } } }"],
        "get `link.tsx` with value dependents" => ["query { file(path: \"link.tsx\") { path dependents(importType: VALUES) { files { items { path } } } } }"],
    );

    Ok(())
}

#[test]
fn test_task_queries() -> Result<(), anyhow::Error> {
    check_json_output!(
        "task_dependencies/query",
        "npm@10.5.0",
        "query",
        "get tasks for app-a" => ["query { package(name: \"app-a\") { tasks { items { name } } } }"],
        "get tasks for lib-b" => ["query { package(name: \"lib-b\") { tasks { items { name } } } }"],
        "get tasks for app-a with dependencies" => ["query { package(name: \"app-a\") { tasks { items { fullName directDependencies { items { fullName } } } } } }"],
        "get tasks for lib-b with dependents" => ["query { package(name: \"lib-b\") { tasks { items { fullName directDependents { items { fullName } } } } } }"],
        "get tasks for app-a with dependencies and dependents" => ["query { package(name: \"app-a\") { tasks { items { fullName allDependencies { items { fullName } } } } } }"],
        "get tasks for lib-b with dependents and dependencies" => ["query { package(name: \"lib-b\") { tasks { items { fullName allDependents { items { fullName } } } } } }"],
        "get tasks for app-a with indirect dependencies" => ["query { package(name: \"app-a\") { tasks { items { fullName indirectDependencies { items { fullName } } } } } }"],
        "get tasks for lib-b with indirect dependents" => ["query { package(name: \"lib-b\") { tasks { items { fullName indirectDependents { items { fullName } } } } } }"],
    );

    Ok(())
}
